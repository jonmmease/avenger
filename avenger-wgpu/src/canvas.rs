use std::{sync::Arc, time::Instant};

use avenger_common::{canvas::CanvasDimensions, types::LinearScaleAdjustment};
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark, area::SceneAreaMark, group::Clip, group::SceneGroup,
        image::SceneImageMark, line::SceneLineMark, mark::SceneMark, path::ScenePathMark,
        rect::SceneRectMark, rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark,
        trail::SceneTrailMark,
    },
    scene_graph::SceneGraph,
};
use itertools::izip;
use wgpu::{
    Adapter, CommandBuffer, CommandEncoderDescriptor, Device, DeviceDescriptor, Extent3d,
    PowerPreference, Queue, RequestAdapterError, RequestAdapterOptions, Surface,
    SurfaceConfiguration, TextureDescriptor, TextureDimension, TextureFormat,
    TextureFormatFeatureFlags, TextureUsages, TextureView, TextureViewDescriptor, Trace,
};
use winit::{dpi::Size, event::WindowEvent, window::Window};

use crate::{
    error::AvengerWgpuError,
    marks::{
        instanced_mark::{InstancedMarkFingerprint, InstancedMarkRenderer},
        multi::{is_axis_aligned_angle, MultiMarkRenderer},
        symbol::{is_circle_only_symbol_mark, CircleSymbolShader, SymbolShader},
        text::{TextAtlasBuilderTrait, TextAtlasRegistration, TextInstance},
    },
    offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
    readback::TextureReadback,
    renderer::{mark_renderer_counts, AvengerRendererCore},
    target::{AvengerRenderTarget, WHITE_CLEAR},
    zindex_layers::compute_zindex_layers,
};

pub use crate::renderer::{MarkRenderer, ZIndexedMark};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasFrameOverlay {
    pub size: [f32; 2],
    pub resize_width: bool,
    pub resize_height: bool,
    pub handle_thickness: f32,
}

pub type TextBuildCtor = Arc<fn() -> Box<dyn TextAtlasBuilderTrait>>;

pub trait CanvasDimensionUtils {
    fn to_physical_size(&self) -> winit::dpi::PhysicalSize<u32>;
}

impl CanvasDimensionUtils for CanvasDimensions {
    fn to_physical_size(&self) -> winit::dpi::PhysicalSize<u32> {
        winit::dpi::PhysicalSize {
            width: self.to_physical_width(),
            height: self.to_physical_height(),
        }
    }
}

#[derive(Default, Clone)]
pub struct CanvasConfig {
    pub text_builder_ctor: Option<TextBuildCtor>,
}

pub trait Canvas {
    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    );
    fn clear_mark_renderer(&mut self);
    fn device(&self) -> &Device;
    fn queue(&self) -> &Queue;
    fn dimensions(&self) -> CanvasDimensions;

    fn texture_format(&self) -> TextureFormat;

    fn sample_count(&self) -> u32;

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer;

    /// The text atlas shared by every multi-renderer on this canvas. Glyphs are
    /// registered into it during the set_scene mark walk, and it is built + uploaded
    /// once per frame at render time.
    fn text_atlas_builder(&mut self) -> &mut dyn TextAtlasBuilderTrait;

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>>;

    fn set_current_zindex(&mut self, zindex: i32);

    fn commit_multi_renderer_if_needed(&mut self, new_zindex: i32);

    fn get_current_zindex(&self) -> i32;

    fn add_arc_mark(
        &mut self,
        mark: &SceneArcMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_arc_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_path_mark(
        &mut self,
        mark: &ScenePathMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_path_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_line_mark(
        &mut self,
        mark: &SceneLineMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_line_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_trail_mark(
        &mut self,
        mark: &SceneTrailMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_trail_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_area_mark(
        &mut self,
        mark: &SceneAreaMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_area_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_symbol_mark(
        &mut self,
        mark: &SceneSymbolMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        if symbol_mark_is_instanced_eligible(mark, group_clip) {
            // Check if compatible renderer already exists
            let fingerprint = mark.instanced_fingerprint();
            let renderer = if let Some(renderer) = self.get_instanced_renderer(fingerprint) {
                renderer
            } else if is_circle_only_symbol_mark(mark) {
                let shader = Box::new(CircleSymbolShader::from_symbol_mark(
                    mark,
                    self.dimensions(),
                    origin,
                ));

                let renderer = Arc::new(InstancedMarkRenderer::new(
                    self.device(),
                    self.texture_format(),
                    self.sample_count(),
                    shader,
                    group_clip.maybe_clip(mark.clip),
                    self.dimensions().scale,
                ));
                renderer
            } else {
                let shader = Box::new(SymbolShader::from_symbol_mark(
                    mark,
                    self.dimensions(),
                    origin,
                )?);

                let renderer = Arc::new(InstancedMarkRenderer::new(
                    self.device(),
                    self.texture_format(),
                    self.sample_count(),
                    shader,
                    group_clip.maybe_clip(mark.clip),
                    self.dimensions().scale,
                ));
                renderer
            };

            self.add_instanced_mark_renderer(
                renderer,
                fingerprint,
                mark.x_adjustment,
                mark.y_adjustment,
            );
        } else {
            self.get_multi_renderer()
                .add_symbol_mark(mark, origin, group_clip)?;
        }

        Ok(())
    }

    fn add_rect_mark(
        &mut self,
        mark: &SceneRectMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_rect_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_rule_mark(
        &mut self,
        mark: &SceneRuleMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_rule_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_text_mark(
        &mut self,
        mark: &SceneTextMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        // Register every glyph run into the shared (per-canvas) text atlas. This
        // mirrors the loop that previously lived in `MultiMarkRenderer::add_text_mark`;
        // only the location of the call moved (the register_text math is unchanged), so
        // glyph bitmaps and baked UVs — and therefore rendered pixels — are identical.
        let dimensions = self.dimensions();
        let text_atlas_builder = self.text_atlas_builder();
        let registrations: Vec<TextAtlasRegistration> = izip!(
            mark.text_iter(),
            mark.x_iter(),
            mark.y_iter(),
            mark.color_iter(),
            mark.align_iter(),
            mark.angle_iter(),
            mark.baseline_iter(),
            mark.font_iter(),
            mark.font_size_iter(),
            mark.font_weight_iter(),
            mark.font_style_iter(),
            mark.limit_iter(),
        )
        .map(
            |(
                text,
                x,
                y,
                color,
                align,
                angle,
                baseline,
                font,
                font_size,
                font_weight,
                font_style,
                limit,
            )| {
                let use_nearest_filter = is_axis_aligned_angle(*angle);
                let instance = TextInstance {
                    text,
                    position: [*x + origin[0], *y + origin[1]],
                    color: &color.color_or_transparent(),
                    align,
                    angle: *angle,
                    baseline,
                    font,
                    font_size: *font_size,
                    font_weight,
                    font_style,
                    limit: *limit,
                    use_nearest_filter,
                };
                text_atlas_builder.register_text(instance, dimensions)
            },
        )
        .collect::<Result<Vec<_>, AvengerWgpuError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        // The two borrows above (`text_atlas_builder`) and below (`get_multi_renderer`)
        // are sequential — `registrations` is owned — so there is no borrow conflict.
        self.get_multi_renderer()
            .add_text_registrations(registrations, group_clip, mark.clip)?;
        Ok(())
    }

    fn add_image_mark(
        &mut self,
        mark: &SceneImageMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_image_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_group_mark(
        &mut self,
        group: &SceneGroup,
        parent_origin: [f32; 2],
        parent_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        // Save parent z-index
        let saved_zindex = self.get_current_zindex();

        // Group's z-index defaults to parent's z-index
        let group_zindex = group.zindex.unwrap_or(saved_zindex);
        self.set_current_zindex(group_zindex);

        // Maybe add rect around group boundary
        if let Some(rect) = group.make_path_mark() {
            self.add_path_mark(&rect, parent_origin, &group.clip)?;
        }

        // Compute new origin
        let origin = [
            parent_origin[0] + group.origin[0],
            parent_origin[1] + group.origin[1],
        ];

        // Compute new clip
        let clip = if let Clip::None = group.clip {
            // No clip defined for this group, propagate parent clip down
            parent_clip.clone()
        } else {
            // Translate clip to absolute coordinates
            group.clip.translate(origin[0], origin[1])
        };

        for mark in &group.marks {
            // Mark inherits group's z-index if it doesn't have its own
            let mark_zindex = mark.zindex().unwrap_or(group_zindex);
            self.set_current_zindex(mark_zindex);

            match mark {
                SceneMark::Arc(mark) => {
                    self.add_arc_mark(mark, origin, &clip)?;
                }
                SceneMark::Symbol(mark) => {
                    self.add_symbol_mark(mark, origin, &clip)?;
                }
                SceneMark::Rect(mark) => {
                    self.add_rect_mark(mark, origin, &clip)?;
                }
                SceneMark::Rule(mark) => {
                    self.add_rule_mark(mark, origin, &clip)?;
                }
                SceneMark::Path(mark) => {
                    self.add_path_mark(mark, origin, &clip)?;
                }
                SceneMark::Line(mark) => {
                    self.add_line_mark(mark, origin, &clip)?;
                }
                SceneMark::Trail(mark) => {
                    self.add_trail_mark(mark, origin, &clip)?;
                }
                SceneMark::Area(mark) => {
                    self.add_area_mark(mark, origin, &clip)?;
                }
                SceneMark::Text(mark) => {
                    self.add_text_mark(mark, origin, &clip)?;
                }
                SceneMark::Image(mark) => {
                    self.add_image_mark(mark, origin, &clip)?;
                }
                SceneMark::Group(group) => {
                    self.add_group_mark(group, origin, &clip)?;
                }
            }
        }

        // Restore previous z-index
        self.set_current_zindex(saved_zindex);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn set_scene(&mut self, scene_graph: &SceneGraph) -> Result<(), AvengerWgpuError> {
        let start = Instant::now();
        // Clear existing marks
        self.clear_mark_renderer();

        // Process groups in document order - z-index sorting will happen during rendering
        let groups = scene_graph.groups();
        let group_count = groups.len();
        for group in groups {
            self.add_group_mark(group, scene_graph.origin, &Clip::None)?;
        }

        tracing::debug!(
            target: "avenger_wgpu::resize",
            set_scene_ms = start.elapsed().as_secs_f64() * 1000.0,
            group_count,
            scene_width = scene_graph.width,
            scene_height = scene_graph.height,
            "wgpu.set_scene"
        );
        Ok(())
    }
}

fn symbol_mark_is_instanced_eligible(mark: &SceneSymbolMark, group_clip: &Clip) -> bool {
    mark.len >= 100
        && mark.gradients.is_empty()
        && matches!(group_clip, Clip::None | Clip::Rect { .. })
}

#[cfg(test)]
mod tests {
    use avenger_color::{Gradient, GradientStop, LinearGradient};
    use avenger_scenegraph::marks::{group::Clip, symbol::SceneSymbolMark};

    use super::symbol_mark_is_instanced_eligible;

    fn large_symbol_mark() -> SceneSymbolMark {
        SceneSymbolMark {
            len: 100,
            ..Default::default()
        }
    }

    #[test]
    fn large_symbol_mark_with_no_clip_is_instanced_eligible() {
        assert!(symbol_mark_is_instanced_eligible(
            &large_symbol_mark(),
            &Clip::None
        ));
    }

    #[test]
    fn stroked_symbol_mark_is_instanced_eligible() {
        let mark = SceneSymbolMark {
            stroke_width: Some(2.0),
            ..large_symbol_mark()
        };

        assert!(symbol_mark_is_instanced_eligible(&mark, &Clip::None));
    }

    #[test]
    fn gradient_symbol_mark_is_not_instanced_eligible() {
        let mark = SceneSymbolMark {
            gradients: vec![Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: [0.0, 0.0, 0.0, 1.0],
                    },
                    GradientStop {
                        offset: 1.0,
                        color: [1.0, 1.0, 1.0, 1.0],
                    },
                ],
            })],
            ..large_symbol_mark()
        };

        assert!(!symbol_mark_is_instanced_eligible(&mark, &Clip::None));
    }

    #[test]
    fn path_clipped_symbol_mark_is_not_instanced_eligible() {
        assert!(!symbol_mark_is_instanced_eligible(
            &large_symbol_mark(),
            &Clip::Path(lyon::path::Path::default()),
        ));
    }
}

// Private shared canvas logic
#[allow(dead_code)]
pub(crate) fn make_background_command<C: Canvas>(
    canvas: &C,
    texture_view: &TextureView,
    resolve_target: Option<&TextureView>,
) -> CommandBuffer {
    let dimensions = canvas.dimensions();
    let extent = Extent3d {
        width: dimensions.to_physical_width(),
        height: dimensions.to_physical_height(),
        depth_or_array_layers: 1,
    };
    let target = if let Some(resolve_target) = resolve_target {
        AvengerRenderTarget::multisampled(
            texture_view,
            resolve_target,
            extent,
            canvas.texture_format(),
            canvas.sample_count(),
            WHITE_CLEAR,
        )
    } else {
        AvengerRenderTarget::new(
            texture_view,
            extent,
            canvas.texture_format(),
            1,
            WHITE_CLEAR,
        )
    };
    AvengerRendererCore::make_background_command_for_target(canvas.device(), target)
}

pub(crate) fn make_wgpu_instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    })
}

pub(crate) async fn make_wgpu_adapter(
    instance: &wgpu::Instance,
    compatible_surface: Option<&Surface<'_>>,
) -> Result<Adapter, AvengerWgpuError> {
    let primary_options = RequestAdapterOptions {
        power_preference: PowerPreference::default(),
        compatible_surface,
        force_fallback_adapter: false,
    };

    match instance.request_adapter(&primary_options).await {
        Ok(adapter) => return Ok(adapter),
        Err(RequestAdapterError::NotFound { .. }) => {}
        Err(_) => return Err(AvengerWgpuError::MakeWgpuAdapterError),
    }

    let fallback_options = RequestAdapterOptions {
        power_preference: PowerPreference::LowPower,
        compatible_surface,
        force_fallback_adapter: true,
    };

    match instance.request_adapter(&fallback_options).await {
        Ok(adapter) => Ok(adapter),
        Err(_) => Err(AvengerWgpuError::MakeWgpuAdapterError),
    }
}

pub(crate) async fn request_wgpu_device(
    adapter: &Adapter,
) -> Result<(Device, Queue), AvengerWgpuError> {
    Ok(adapter
        .request_device(&DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            // WebGL doesn't support all of wgpu's features, so if
            // we're building for the web we'll have to disable some.
            required_limits: if cfg!(target_arch = "wasm32") {
                wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                wgpu::Limits::default()
            },
            memory_hints: wgpu::MemoryHints::Performance,
            trace: Trace::Off,
        })
        .await?)
}

pub(crate) fn create_multisampled_framebuffer(
    device: &Device,
    width: u32,
    height: u32,
    format: TextureFormat,
    sample_count: u32,
) -> TextureView {
    let multisampled_texture_extent = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let multisampled_frame_descriptor = &TextureDescriptor {
        size: multisampled_texture_extent,
        mip_level_count: 1,
        sample_count,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT,
        label: None,
        view_formats: &[],
    };

    device
        .create_texture(multisampled_frame_descriptor)
        .create_view(&TextureViewDescriptor::default())
}

pub(crate) fn get_supported_sample_count(sample_flags: TextureFormatFeatureFlags) -> u32 {
    // Get max supported sample count up to 4
    if sample_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4) {
        4
    } else if sample_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X2) {
        2
    } else {
        1
    }
}

pub struct WindowCanvas<'window> {
    surface_config: SurfaceConfiguration,
    renderer: AvengerRendererCore,
    frame_overlay: Option<CanvasFrameOverlay>,

    // Order of properties determines drop order.
    // Device must be dropped after the buffers and textures associated with marks
    multisampled_framebuffer: TextureView,
    queue: Queue,
    device: Device,
    surface: Surface<'window>,
    window: Arc<Window>,
}

impl WindowCanvas<'_> {
    pub async fn new(
        window: Window,
        dimensions: CanvasDimensions,
        config: CanvasConfig,
    ) -> Result<Self, AvengerWgpuError> {
        let requested_size = dimensions.to_physical_size();
        let accepted_size = window
            .request_inner_size(Size::Physical(requested_size))
            .unwrap_or_else(|| window.inner_size());
        let dimensions = CanvasDimensions {
            size: [
                accepted_size.width as f32 / dimensions.scale,
                accepted_size.height as f32 / dimensions.scale,
            ],
            scale: dimensions.scale,
        };
        let instance = make_wgpu_instance();
        let window = Arc::new(window);
        let surface = instance.create_surface(window.clone())?;
        let adapter = make_wgpu_adapter(&instance, Some(&surface)).await?;
        let (device, queue) = request_wgpu_device(&adapter).await?;

        let surface_caps = surface.get_capabilities(&adapter);

        // Select first non-srgb texture format
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: dimensions.to_physical_width(),
            height: dimensions.to_physical_height(),
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let sample_count = 1;
        let multisampled_framebuffer = create_multisampled_framebuffer(
            &device,
            surface_config.width,
            surface_config.height,
            surface_format,
            sample_count,
        );

        // // Uncomment to capture GPU boundary
        // unsafe { device.start_graphics_debugger_capture() };

        let renderer =
            AvengerRendererCore::new(&device, dimensions, surface_format, sample_count, config);

        Ok(Self {
            surface,
            device,
            queue,
            multisampled_framebuffer,
            surface_config,
            window,
            renderer,
            frame_overlay: None,
        })
    }

    pub fn get_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.renderer.dimensions().to_physical_size()
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn set_frame_overlay(&mut self, overlay: Option<CanvasFrameOverlay>) {
        self.frame_overlay = overlay;
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.update_physical_size(new_size.width, new_size.height);
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    fn update_physical_size(&mut self, width: u32, height: u32) {
        let scale = self.renderer.dimensions().scale;
        self.renderer.set_dimensions(CanvasDimensions {
            size: [width as f32 / scale, height as f32 / scale],
            scale,
        });

        self.surface_config.width = width;
        self.surface_config.height = height;
        self.multisampled_framebuffer = create_multisampled_framebuffer(
            &self.device,
            width,
            height,
            self.surface_config.format,
            self.renderer.sample_count(),
        );
    }

    fn sync_to_acquired_surface_texture(&mut self, texture_extent: Extent3d) {
        if texture_extent.width == 0 || texture_extent.height == 0 {
            return;
        }

        if self.surface_config.width == texture_extent.width
            && self.surface_config.height == texture_extent.height
        {
            return;
        }

        self.update_physical_size(texture_extent.width, texture_extent.height);
    }

    #[allow(unused_variables)]
    pub fn input(&mut self, event: &WindowEvent) -> bool {
        false
    }

    pub fn update(&mut self) {}

    pub fn render(&mut self) -> Result<(), AvengerWgpuError> {
        let _span = tracing::debug_span!("wgpu.render").entered();
        let render_start = Instant::now();
        let output = self.surface.get_current_texture()?;
        let output_extent = output.texture.size();
        self.sync_to_acquired_surface_texture(output_extent);
        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());
        let sample_count = self.renderer.sample_count();
        let render_target_extent = if sample_count > 1 {
            Extent3d {
                width: self.surface_config.width,
                height: self.surface_config.height,
                depth_or_array_layers: 1,
            }
        } else {
            output_extent
        };

        // Commit open multi-renderer
        self.renderer.commit_all_multi_renderers();
        let marks = self.renderer.marks().to_vec();

        // Collect z-indices and compute layers
        let zindices: Vec<i32> = marks.iter().map(|m| m.zindex).collect();
        let layers = if zindices.is_empty() {
            vec![]
        } else {
            compute_zindex_layers(zindices)
        };
        let layer_count = layers.len();
        let (instanced_renderer_count, multi_renderer_count) = mark_renderer_counts(&marks);

        // Render background first
        let command_build_start = Instant::now();
        let background_target = if sample_count > 1 {
            AvengerRenderTarget::multisampled(
                &self.multisampled_framebuffer,
                &view,
                render_target_extent,
                self.renderer.texture_format(),
                sample_count,
                WHITE_CLEAR,
            )
        } else {
            AvengerRenderTarget::swapchain(
                &view,
                render_target_extent,
                self.renderer.texture_format(),
                WHITE_CLEAR,
            )
        };
        let background_command = self
            .renderer
            .make_background_command(&self.device, background_target);
        let mut commands = vec![background_command];

        let multi_render_resources = self.renderer.multi_render_resources().clone();

        // Build the shared text atlas bind groups ONCE per frame (instead of once per
        // multi-renderer). Every renderer this frame registered into the same atlas, so
        // these bind groups are page-correct for all of them.
        let text_bind_groups = self
            .renderer
            .build_text_bind_groups(&self.device, &self.queue);

        // Prepare the single shared multi-renderer ONCE per frame (uniform, gradient/
        // image atlas bind groups, stencil buffer, combined vertex/index/clip buffers).
        // Each z-run (a `MarkRenderer::Multi` batch range) is then encoded from this
        // shared prep, so the 40-ish per-cell renderers' setup collapses to one.
        let _prepare_start = Instant::now();
        let prepared = self.renderer.shared_multi_mut().prepare(
            &self.device,
            &self.queue,
            render_target_extent,
            &multi_render_resources,
        );
        tracing::debug!(
            target: "avenger_wgpu::resize",
            prepare_ms = _prepare_start.elapsed().as_secs_f64() * 1000.0,
            "wgpu.prepare"
        );

        // Render marks by layer
        // Coalesce consecutive multi-mark runs (in (layer, document) order) into one
        // command buffer with merged render passes; instanced marks break a run and
        // render on their own pipeline, interleaved.
        let mut mark_encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Avenger Mark Render Encoder"),
            });
        let mut encoded_marks = false;
        let mut pending: Vec<std::ops::Range<usize>> = Vec::new();
        for (min_z, max_z) in layers {
            for mark in &marks {
                if mark.zindex >= min_z && mark.zindex <= max_z {
                    match &mark.renderer {
                        MarkRenderer::Multi { batch_range } => pending.push(batch_range.clone()),
                        MarkRenderer::Instanced {
                            renderer,
                            x_adjustment,
                            y_adjustment,
                        } => {
                            if !pending.is_empty() {
                                if sample_count > 1 {
                                    self.renderer.shared_multi().encode_multi_ranges_into(
                                        &mut mark_encoder,
                                        render_target_extent,
                                        &self.multisampled_framebuffer,
                                        Some(&view),
                                        &multi_render_resources,
                                        &text_bind_groups,
                                        &prepared,
                                        &pending,
                                    );
                                } else {
                                    self.renderer.shared_multi().encode_multi_ranges_into(
                                        &mut mark_encoder,
                                        render_target_extent,
                                        &view,
                                        None,
                                        &multi_render_resources,
                                        &text_bind_groups,
                                        &prepared,
                                        &pending,
                                    );
                                };
                                pending.clear();
                            }
                            if sample_count > 1 {
                                renderer.encode_into(
                                    &self.device,
                                    &mut mark_encoder,
                                    &self.multisampled_framebuffer,
                                    Some(&view),
                                    *x_adjustment,
                                    *y_adjustment,
                                );
                            } else {
                                renderer.encode_into(
                                    &self.device,
                                    &mut mark_encoder,
                                    &view,
                                    None,
                                    *x_adjustment,
                                    *y_adjustment,
                                );
                            };
                            encoded_marks = true;
                        }
                    }
                }
            }
        }
        if !pending.is_empty() {
            if sample_count > 1 {
                self.renderer.shared_multi().encode_multi_ranges_into(
                    &mut mark_encoder,
                    render_target_extent,
                    &self.multisampled_framebuffer,
                    Some(&view),
                    &multi_render_resources,
                    &text_bind_groups,
                    &prepared,
                    &pending,
                );
            } else {
                self.renderer.shared_multi().encode_multi_ranges_into(
                    &mut mark_encoder,
                    render_target_extent,
                    &view,
                    None,
                    &multi_render_resources,
                    &text_bind_groups,
                    &prepared,
                    &pending,
                );
            };
            encoded_marks = true;
        }
        if encoded_marks {
            commands.push(mark_encoder.finish());
        }

        let frame_overlay_command = if sample_count > 1 {
            let overlay_start = Instant::now();
            let command = self.renderer.make_frame_overlay_command(
                &self.device,
                &self.queue,
                self.frame_overlay,
                render_target_extent,
                &self.multisampled_framebuffer,
                Some(&view),
                &text_bind_groups,
            )?;
            tracing::trace!(
                target: "avenger_wgpu::resize",
                overlay_command_ms = overlay_start.elapsed().as_secs_f64() * 1000.0,
                "wgpu.render overlay command"
            );
            command
        } else {
            let overlay_start = Instant::now();
            let command = self.renderer.make_frame_overlay_command(
                &self.device,
                &self.queue,
                self.frame_overlay,
                render_target_extent,
                &view,
                None,
                &text_bind_groups,
            )?;
            tracing::trace!(
                target: "avenger_wgpu::resize",
                overlay_command_ms = overlay_start.elapsed().as_secs_f64() * 1000.0,
                "wgpu.render overlay command"
            );
            command
        };
        if let Some(command) = frame_overlay_command {
            commands.push(command);
        }

        let command_build_elapsed = command_build_start.elapsed();
        let command_count = commands.len();
        let submit_start = Instant::now();
        self.queue.submit(commands);
        output.present();
        let submit_elapsed = submit_start.elapsed();

        tracing::debug!(
            target: "avenger_wgpu::resize",
            render_ms = render_start.elapsed().as_secs_f64() * 1000.0,
            command_build_ms = command_build_elapsed.as_secs_f64() * 1000.0,
            submit_present_ms = submit_elapsed.as_secs_f64() * 1000.0,
            command_count,
            renderer_count = marks.len(),
            instanced_renderer_count,
            multi_renderer_count,
            layer_count,
            "wgpu.render"
        );

        Ok(())
    }
}

impl Canvas for WindowCanvas<'_> {
    fn set_current_zindex(&mut self, zindex: i32) {
        self.renderer.set_current_zindex(zindex);
    }

    fn commit_multi_renderer_if_needed(&mut self, _new_zindex: i32) {
        self.renderer.commit_multi_renderer_if_needed();
    }

    fn get_current_zindex(&self) -> i32 {
        self.renderer.current_zindex()
    }

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.renderer.shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut dyn TextAtlasBuilderTrait {
        self.renderer.text_atlas_builder_mut()
    }

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.renderer.get_instanced_renderer(fingerprint)
    }

    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        self.renderer.add_instanced_mark_renderer(
            mark_renderer,
            fingerprint,
            x_adjustment,
            y_adjustment,
        );
    }

    fn clear_mark_renderer(&mut self) {
        self.renderer.clear_mark_renderer();
    }

    fn device(&self) -> &Device {
        &self.device
    }

    fn queue(&self) -> &Queue {
        &self.queue
    }

    fn dimensions(&self) -> CanvasDimensions {
        self.renderer.dimensions()
    }

    fn texture_format(&self) -> TextureFormat {
        self.renderer.texture_format()
    }

    fn sample_count(&self) -> u32 {
        self.renderer.sample_count()
    }
}

// impl<'window> Drop for WindowCanvas<'window> {
//     fn drop(&mut self) {
//         unsafe { self.device.stop_graphics_debugger_capture() };
//     }
// }

pub struct PngCanvas {
    renderer: AvengerRendererCore,
    output_target: OffscreenTarget,
    readback: TextureReadback,

    // The order of properties in a struct is the order in which items are dropped.
    // wgpu seems to require that the device be dropped last, otherwise there is a resouce
    // leak.
    multisampled_framebuffer: TextureView,
    queue: Queue,
    device: Device,
}

impl PngCanvas {
    #[tracing::instrument(skip_all)]
    pub async fn new(
        dimensions: CanvasDimensions,
        config: CanvasConfig,
    ) -> Result<Self, AvengerWgpuError> {
        let instance = make_wgpu_instance();
        let adapter = make_wgpu_adapter(&instance, None).await?;
        let (device, queue) = request_wgpu_device(&adapter).await?;
        let texture_format = TextureFormat::Rgba8Unorm;
        let format_flags = adapter.get_texture_format_features(texture_format).flags;
        let sample_count = get_supported_sample_count(format_flags);
        let output_target_descriptor = OffscreenTargetDescriptor::new(dimensions, texture_format)
            .with_usage(TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT)
            .with_label("PngCanvas output texture");
        let output_target = OffscreenTarget::new(&device, &output_target_descriptor, 1);
        let readback = TextureReadback::new(&device, output_target.extent);

        let multisampled_framebuffer = create_multisampled_framebuffer(
            &device,
            output_target.extent.width,
            output_target.extent.height,
            texture_format,
            sample_count,
        );

        let renderer =
            AvengerRendererCore::new(&device, dimensions, texture_format, sample_count, config);

        Ok(Self {
            device,
            queue,
            multisampled_framebuffer,
            renderer,
            output_target,
            readback,
        })
    }

    #[tracing::instrument(skip_all)]
    pub async fn render(&mut self) -> Result<image::RgbaImage, AvengerWgpuError> {
        let render_start = Instant::now();
        self.renderer.commit_all_multi_renderers();
        let sample_count = self.renderer.sample_count();
        let dimensions = self.renderer.dimensions();
        let marks = self.renderer.marks().to_vec();

        let zindices: Vec<i32> = marks.iter().map(|m| m.zindex).collect();
        let layers = if zindices.is_empty() {
            vec![]
        } else {
            compute_zindex_layers(zindices)
        };
        let layer_count = layers.len();
        let (instanced_renderer_count, multi_renderer_count) = mark_renderer_counts(&marks);

        let render_target_extent = self.output_target.extent;
        let render_target = if sample_count > 1 {
            AvengerRenderTarget::multisampled(
                &self.multisampled_framebuffer,
                &self.output_target.view,
                render_target_extent,
                self.renderer.texture_format(),
                sample_count,
                WHITE_CLEAR,
            )
        } else {
            self.output_target.render_target(WHITE_CLEAR)
        };

        let command_build_start = Instant::now();
        let commands =
            self.renderer
                .build_frame_commands(&self.device, &self.queue, render_target, None)?;
        let command_build_elapsed = command_build_start.elapsed();
        let command_count = commands.len();

        let submit_start = Instant::now();
        self.queue.submit(commands);
        let submit_elapsed = submit_start.elapsed();

        let extract_start = Instant::now();
        let mut extract_encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Extract Texture Encoder"),
            });
        debug_assert_eq!(self.readback.texture_extent(), self.output_target.extent);
        self.readback
            .encode_copy_from_texture(&mut extract_encoder, &self.output_target.texture);
        self.queue.submit(Some(extract_encoder.finish()));
        let extract_elapsed = extract_start.elapsed();

        let map_read_start = Instant::now();
        let img = self
            .readback
            .read_rgba8(
                &self.device,
                dimensions.to_physical_width(),
                dimensions.to_physical_height(),
            )
            .await?;
        let map_read_elapsed = map_read_start.elapsed();

        tracing::debug!(
            target: "avenger_wgpu::resize",
            render_ms = render_start.elapsed().as_secs_f64() * 1000.0,
            command_build_ms = command_build_elapsed.as_secs_f64() * 1000.0,
            submit_ms = submit_elapsed.as_secs_f64() * 1000.0,
            extract_ms = extract_elapsed.as_secs_f64() * 1000.0,
            map_read_ms = map_read_elapsed.as_secs_f64() * 1000.0,
            physical_width = dimensions.to_physical_width(),
            physical_height = dimensions.to_physical_height(),
            command_count,
            renderer_count = marks.len(),
            instanced_renderer_count,
            multi_renderer_count,
            layer_count,
            "png.render"
        );
        Ok(img)
    }
}

impl Canvas for PngCanvas {
    fn set_current_zindex(&mut self, zindex: i32) {
        self.renderer.set_current_zindex(zindex);
    }

    fn commit_multi_renderer_if_needed(&mut self, _new_zindex: i32) {
        self.renderer.commit_multi_renderer_if_needed();
    }

    fn get_current_zindex(&self) -> i32 {
        self.renderer.current_zindex()
    }

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.renderer.shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut dyn TextAtlasBuilderTrait {
        self.renderer.text_atlas_builder_mut()
    }

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.renderer.get_instanced_renderer(fingerprint)
    }

    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        self.renderer.add_instanced_mark_renderer(
            mark_renderer,
            fingerprint,
            x_adjustment,
            y_adjustment,
        );
    }

    fn clear_mark_renderer(&mut self) {
        self.renderer.clear_mark_renderer();
    }

    fn device(&self) -> &Device {
        &self.device
    }

    fn queue(&self) -> &Queue {
        &self.queue
    }

    fn dimensions(&self) -> CanvasDimensions {
        self.renderer.dimensions()
    }

    fn texture_format(&self) -> TextureFormat {
        self.renderer.texture_format()
    }

    fn sample_count(&self) -> u32 {
        self.renderer.sample_count()
    }
}
