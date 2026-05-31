use std::{collections::HashMap, sync::Arc, time::Instant};

use avenger_common::{
    canvas::CanvasDimensions,
    types::{ColorOrGradient, LinearScaleAdjustment},
    value::ScalarOrArray,
};
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark, area::SceneAreaMark, group::Clip, group::SceneGroup,
        image::SceneImageMark, line::SceneLineMark, mark::SceneMark, path::ScenePathMark,
        rect::SceneRectMark, rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark,
        trail::SceneTrailMark,
    },
    scene_graph::SceneGraph,
};
use image::imageops::crop_imm;
use itertools::izip;
use wgpu::{
    Adapter, BindGroup, Buffer, BufferAddress, BufferDescriptor, BufferUsages, CommandBuffer,
    CommandEncoderDescriptor, Device, DeviceDescriptor, Extent3d, LoadOp, MapMode, Operations,
    Origin3d, PowerPreference, Queue, RenderPassColorAttachment, RenderPassDescriptor,
    RequestAdapterError, RequestAdapterOptions, StoreOp, Surface, SurfaceConfiguration,
    TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureFormatFeatureFlags, TextureUsages,
    TextureView, TextureViewDescriptor, Trace,
};
use winit::{dpi::Size, event::WindowEvent, window::Window};

use crate::{
    error::AvengerWgpuError,
    marks::{
        instanced_mark::{InstancedMarkFingerprint, InstancedMarkRenderer},
        multi::{is_axis_aligned_angle, MultiMarkRenderResources, MultiMarkRenderer},
        symbol::{is_circle_only_symbol_mark, CircleSymbolShader, SymbolShader},
        text::{TextAtlasBuilderTrait, TextAtlasRegistration, TextInstance},
    },
    zindex_layers::compute_zindex_layers,
};

pub enum MarkRenderer {
    Instanced {
        renderer: Arc<InstancedMarkRenderer>,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    },
    /// A contiguous run of multi-marks at one z-index, stored as a half-open range
    /// of batch indices into the canvas's single shared `MultiMarkRenderer`. The
    /// renderer is prepared once per frame; consecutive runs are coalesced and
    /// encoded via `encode_multi_ranges`, preserving exact draw order and clipping.
    Multi { batch_range: std::ops::Range<usize> },
}

/// A mark renderer with its associated z-index
pub struct ZIndexedMark {
    pub zindex: i32,
    pub renderer: MarkRenderer,
}

fn mark_renderer_counts(marks: &[ZIndexedMark]) -> (usize, usize) {
    marks
        .iter()
        .fold((0, 0), |(instanced_count, multi_count), mark| {
            match &mark.renderer {
                MarkRenderer::Instanced { .. } => (instanced_count + 1, multi_count),
                MarkRenderer::Multi { .. } => (instanced_count, multi_count + 1),
            }
        })
}

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
    use avenger_common::types::{Gradient, GradientStop, LinearGradient};
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
pub(crate) fn make_background_command<C: Canvas>(
    canvas: &C,
    texture_view: &TextureView,
    resolve_target: Option<&TextureView>,
) -> CommandBuffer {
    let mut background_encoder =
        canvas
            .device()
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Render Background Encoder"),
            });

    {
        let _render_pass = background_encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("Render Pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: texture_view,
                resolve_target,
                ops: Operations {
                    load: LoadOp::Clear(wgpu::Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    }),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
    }
    background_encoder.finish()
}

/// Construct a text atlas builder, shared by a single canvas across all of its
/// multi-renderers. This mirrors the construction logic that previously lived in
/// `MultiMarkRenderer::new`: honor a caller-supplied `text_builder_ctor`, otherwise
/// fall back to the cosmic-text rasterizer (native), the html-canvas rasterizer
/// (wasm), or the null builder (text disabled).
pub(crate) fn make_text_atlas_builder(
    text_builder_ctor: &Option<TextBuildCtor>,
) -> Box<dyn TextAtlasBuilderTrait> {
    if let Some(text_builder_ctor) = text_builder_ctor {
        text_builder_ctor()
    } else {
        cfg_if::cfg_if! {
            if #[cfg(feature = "cosmic-text")] {
                use crate::marks::text::TextAtlasBuilder;
                use std::sync::Arc;
                let inner_text_atlas_builder: Box<dyn TextAtlasBuilderTrait> = Box::new(TextAtlasBuilder::new(Arc::new(
                    avenger_text::rasterization::cosmic::CosmicTextRasterizer::<crate::marks::text::GlyphBBoxAndAtlasCoords>::new())
                ));
            } else if #[cfg(target_arch = "wasm32")] {
                use crate::marks::text::TextAtlasBuilder;
                use std::sync::Arc;
                let inner_text_atlas_builder: Box<dyn TextAtlasBuilderTrait> = Box::new(TextAtlasBuilder::new(Arc::new(
                    avenger_text::rasterization::html_canvas::HtmlCanvasTextRasterizer::<crate::marks::text::GlyphBBoxAndAtlasCoords>::new())
                ));
            } else {
                use crate::marks::text::NullTextAtlasBuilder;
                let inner_text_atlas_builder: Box<dyn TextAtlasBuilderTrait> = Box::new(NullTextAtlasBuilder);
            }
        };
        inner_text_atlas_builder
    }
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
    sample_count: u32,
    surface_config: SurfaceConfiguration,
    dimensions: CanvasDimensions,
    marks: Vec<ZIndexedMark>,
    // All multi-marks accumulate into one shared renderer; each z-run is recorded
    // in `marks` as a batch range. `run_start` is the batch index where the current
    // (uncommitted) z-run began.
    shared_multi: MultiMarkRenderer,
    run_start: usize,
    current_zindex: i32,
    instanced_renderers: HashMap<u64, Arc<InstancedMarkRenderer>>,
    multi_render_resources: MultiMarkRenderResources,
    config: CanvasConfig,
    frame_overlay: Option<CanvasFrameOverlay>,
    // Text atlas shared by all multi-renderers; built + uploaded once per frame.
    text_atlas_builder: Box<dyn TextAtlasBuilderTrait>,

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

        let multi_render_resources =
            MultiMarkRenderResources::new(&device, surface_format, sample_count);

        let text_atlas_builder = make_text_atlas_builder(&config.text_builder_ctor);

        Ok(Self {
            surface,
            device,
            queue,
            multisampled_framebuffer,
            sample_count,
            surface_config,
            dimensions,
            window,
            marks: Vec::new(),
            shared_multi: MultiMarkRenderer::new(dimensions),
            run_start: 0,
            current_zindex: 0,
            instanced_renderers: HashMap::new(),
            multi_render_resources,
            config,
            frame_overlay: None,
            text_atlas_builder,
        })
    }

    pub fn get_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.dimensions.to_physical_size()
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn set_frame_overlay(&mut self, overlay: Option<CanvasFrameOverlay>) {
        self.frame_overlay = overlay;
    }

    fn commit_all_multi_renderers(&mut self) {
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(ZIndexedMark {
                zindex: self.current_zindex,
                renderer: MarkRenderer::Multi {
                    batch_range: self.run_start..end,
                },
            });
        }
        self.run_start = end;
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.update_physical_size(new_size.width, new_size.height);
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    fn update_physical_size(&mut self, width: u32, height: u32) {
        self.dimensions = CanvasDimensions {
            size: [
                width as f32 / self.dimensions.scale,
                height as f32 / self.dimensions.scale,
            ],
            scale: self.dimensions.scale,
        };

        self.surface_config.width = width;
        self.surface_config.height = height;
        self.multisampled_framebuffer = create_multisampled_framebuffer(
            &self.device,
            width,
            height,
            self.surface_config.format,
            self.sample_count,
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

    fn make_frame_overlay_command(
        &self,
        _texture_format: TextureFormat,
        render_target_extent: Extent3d,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
        text_bind_groups: &[BindGroup],
    ) -> Result<Option<CommandBuffer>, AvengerWgpuError> {
        let Some(overlay) = self.frame_overlay else {
            return Ok(None);
        };

        let width = overlay.size[0].max(0.0);
        let height = overlay.size[1].max(0.0);
        if width <= 0.0 || height <= 0.0 {
            return Ok(None);
        }

        let border = 1.0;
        let handle = overlay.handle_thickness.max(border);
        let outline = ColorOrGradient::Color([0.12, 0.15, 0.18, 0.95]);
        let handle_fill = ColorOrGradient::Color([0.12, 0.15, 0.18, 0.16]);

        let mut x = vec![0.0, 0.0, 0.0, (width - border).max(0.0)];
        let mut y = vec![0.0, (height - border).max(0.0), 0.0, 0.0];
        let mut rect_width = vec![width, width, border, border];
        let mut rect_height = vec![border, border, height, height];
        let mut fill = vec![
            outline.clone(),
            outline.clone(),
            outline.clone(),
            outline.clone(),
        ];

        if overlay.resize_width {
            x.push((width - handle).max(0.0));
            y.push(0.0);
            rect_width.push(handle);
            rect_height.push(height);
            fill.push(handle_fill.clone());
        }

        if overlay.resize_height {
            x.push(0.0);
            y.push((height - handle).max(0.0));
            rect_width.push(width);
            rect_height.push(handle);
            fill.push(handle_fill);
        }

        let mark = SceneRectMark {
            name: "canvas_frame_overlay".to_string(),
            clip: false,
            len: x.len() as u32,
            gradients: Vec::new(),
            x: ScalarOrArray::new_array(x),
            y: ScalarOrArray::new_array(y),
            width: Some(ScalarOrArray::new_array(rect_width)),
            height: Some(ScalarOrArray::new_array(rect_height)),
            x2: None,
            y2: None,
            fill: ScalarOrArray::new_array(fill),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::transparent()),
            stroke_width: ScalarOrArray::new_scalar(0.0),
            corner_radius: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: None,
        };

        let mut renderer = MultiMarkRenderer::new(self.dimensions);
        renderer.add_rect_mark(&mark, [0.0, 0.0], &Clip::None)?;

        // The overlay has no text, but `render_with_resources` indexes
        // `text_bind_groups[0]`; reuse the frame's shared text bind groups.
        Ok(Some(renderer.render_with_resources(
            &self.device,
            &self.queue,
            render_target_extent,
            texture_view,
            resolve_target,
            &self.multi_render_resources,
            text_bind_groups,
        )))
    }

    pub fn render(&mut self) -> Result<(), AvengerWgpuError> {
        let _span = tracing::debug_span!("wgpu.render").entered();
        let render_start = Instant::now();
        let output = self.surface.get_current_texture()?;
        let output_extent = output.texture.size();
        self.sync_to_acquired_surface_texture(output_extent);
        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());
        let render_target_extent = if self.sample_count > 1 {
            Extent3d {
                width: self.surface_config.width,
                height: self.surface_config.height,
                depth_or_array_layers: 1,
            }
        } else {
            output_extent
        };

        // Commit open multi-renderer
        self.commit_all_multi_renderers();

        // Collect z-indices and compute layers
        let zindices: Vec<i32> = self.marks.iter().map(|m| m.zindex).collect();
        let layers = if zindices.is_empty() {
            vec![]
        } else {
            compute_zindex_layers(zindices)
        };
        let layer_count = layers.len();
        let (instanced_renderer_count, multi_renderer_count) = mark_renderer_counts(&self.marks);

        // Render background first
        let command_build_start = Instant::now();
        let background_command = if self.sample_count > 1 {
            make_background_command(self, &self.multisampled_framebuffer, Some(&view))
        } else {
            make_background_command(self, &view, None)
        };
        let mut commands = vec![background_command];

        let texture_format = self.texture_format();
        let multi_render_resources = self.multi_render_resources.clone();

        // Build the shared text atlas bind groups ONCE per frame (instead of once per
        // multi-renderer). Every renderer this frame registered into the same atlas, so
        // these bind groups are page-correct for all of them.
        let (text_atlas_size, text_atlas_images) = self.text_atlas_builder.build();
        let text_bind_groups = MultiMarkRenderer::make_text_bind_groups_dual_sampler(
            &self.device,
            &self.queue,
            self.multi_render_resources.text_layout(),
            text_atlas_size,
            &text_atlas_images,
        );

        // Prepare the single shared multi-renderer ONCE per frame (uniform, gradient/
        // image atlas bind groups, stencil buffer, combined vertex/index/clip buffers).
        // Each z-run (a `MarkRenderer::Multi` batch range) is then encoded from this
        // shared prep, so the 40-ish per-cell renderers' setup collapses to one.
        let _prepare_start = Instant::now();
        let prepared = self.shared_multi.prepare(
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
        let mut pending: Vec<std::ops::Range<usize>> = Vec::new();
        for (min_z, max_z) in layers {
            for mark in &self.marks {
                if mark.zindex >= min_z && mark.zindex <= max_z {
                    match &mark.renderer {
                        MarkRenderer::Multi { batch_range } => pending.push(batch_range.clone()),
                        MarkRenderer::Instanced {
                            renderer,
                            x_adjustment,
                            y_adjustment,
                        } => {
                            if !pending.is_empty() {
                                let c = if self.sample_count > 1 {
                                    self.shared_multi.encode_multi_ranges(
                                        &self.device,
                                        render_target_extent,
                                        &self.multisampled_framebuffer,
                                        Some(&view),
                                        &multi_render_resources,
                                        &text_bind_groups,
                                        &prepared,
                                        &pending,
                                    )
                                } else {
                                    self.shared_multi.encode_multi_ranges(
                                        &self.device,
                                        render_target_extent,
                                        &view,
                                        None,
                                        &multi_render_resources,
                                        &text_bind_groups,
                                        &prepared,
                                        &pending,
                                    )
                                };
                                commands.push(c);
                                pending.clear();
                            }
                            let c = if self.sample_count > 1 {
                                renderer.render(
                                    &self.device,
                                    &self.multisampled_framebuffer,
                                    Some(&view),
                                    *x_adjustment,
                                    *y_adjustment,
                                )
                            } else {
                                renderer.render(
                                    &self.device,
                                    &view,
                                    None,
                                    *x_adjustment,
                                    *y_adjustment,
                                )
                            };
                            commands.push(c);
                        }
                    }
                }
            }
        }
        if !pending.is_empty() {
            let c = if self.sample_count > 1 {
                self.shared_multi.encode_multi_ranges(
                    &self.device,
                    render_target_extent,
                    &self.multisampled_framebuffer,
                    Some(&view),
                    &multi_render_resources,
                    &text_bind_groups,
                    &prepared,
                    &pending,
                )
            } else {
                self.shared_multi.encode_multi_ranges(
                    &self.device,
                    render_target_extent,
                    &view,
                    None,
                    &multi_render_resources,
                    &text_bind_groups,
                    &prepared,
                    &pending,
                )
            };
            commands.push(c);
        }

        let frame_overlay_command = if self.sample_count > 1 {
            let overlay_start = Instant::now();
            let command = self.make_frame_overlay_command(
                texture_format,
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
            let command = self.make_frame_overlay_command(
                texture_format,
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
            renderer_count = self.marks.len(),
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
        if zindex != self.current_zindex {
            self.commit_multi_renderer_if_needed(zindex);
        }
        self.current_zindex = zindex;
    }

    fn commit_multi_renderer_if_needed(&mut self, _new_zindex: i32) {
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(ZIndexedMark {
                zindex: self.current_zindex,
                renderer: MarkRenderer::Multi {
                    batch_range: self.run_start..end,
                },
            });
        }
        self.run_start = end;
    }

    fn get_current_zindex(&self) -> i32 {
        self.current_zindex
    }

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        &mut self.shared_multi
    }

    fn text_atlas_builder(&mut self) -> &mut dyn TextAtlasBuilderTrait {
        &mut *self.text_atlas_builder
    }

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.instanced_renderers.get(&fingerprint).cloned()
    }

    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(ZIndexedMark {
                zindex: self.current_zindex,
                renderer: MarkRenderer::Multi {
                    batch_range: self.run_start..end,
                },
            });
        }
        self.run_start = end;
        self.instanced_renderers
            .insert(fingerprint, mark_renderer.clone());
        self.marks.push(ZIndexedMark {
            zindex: self.current_zindex,
            renderer: MarkRenderer::Instanced {
                renderer: mark_renderer,
                x_adjustment,
                y_adjustment,
            },
        });
    }

    fn clear_mark_renderer(&mut self) {
        // One shared multi-renderer per canvas: reset it in place each frame and
        // clear the recorded z-run marks. (Instanced fingerprint cache is retained.)
        self.shared_multi.reset_for_frame(self.dimensions);
        self.run_start = 0;
        self.marks.clear();

        // Reset the shared text atlas so each frame starts clean (matches the old
        // per-renderer reset-per-frame semantics). `TextAtlasBuilder` has no reset
        // method, so replace it with a fresh builder via the same ctor.
        self.text_atlas_builder = make_text_atlas_builder(&self.config.text_builder_ctor);
    }

    fn device(&self) -> &Device {
        &self.device
    }

    fn queue(&self) -> &Queue {
        &self.queue
    }

    fn dimensions(&self) -> CanvasDimensions {
        self.dimensions
    }

    fn texture_format(&self) -> TextureFormat {
        self.surface_config.format
    }

    fn sample_count(&self) -> u32 {
        self.sample_count
    }
}

// impl<'window> Drop for WindowCanvas<'window> {
//     fn drop(&mut self) {
//         unsafe { self.device.stop_graphics_debugger_capture() };
//     }
// }

pub struct PngCanvas {
    sample_count: u32,
    marks: Vec<ZIndexedMark>,
    current_zindex: i32,
    dimensions: CanvasDimensions,
    texture_view: TextureView,
    output_buffer: Buffer,
    texture: Texture,
    texture_size: Extent3d,
    padded_width: u32,
    padded_height: u32,
    shared_multi: MultiMarkRenderer,
    run_start: usize,
    instanced_renderers: HashMap<u64, Arc<InstancedMarkRenderer>>,
    multi_render_resources: MultiMarkRenderResources,
    config: CanvasConfig,
    // Text atlas shared by all multi-renderers; built + uploaded once per frame.
    text_atlas_builder: Box<dyn TextAtlasBuilderTrait>,

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
        let texture_desc = TextureDescriptor {
            size: Extent3d {
                width: dimensions.to_physical_width(),
                height: dimensions.to_physical_height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1, // Sample count of output texture is always 1
            dimension: TextureDimension::D2,
            format: texture_format,
            usage: TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT,
            label: None,
            view_formats: &[texture_format],
        };
        let texture_size = texture_desc.size;
        let texture = device.create_texture(&texture_desc);
        let texture_view = texture.create_view(&Default::default());

        // we need to store this for later
        let u32_size = std::mem::size_of::<u32>() as u32;

        // Width and height must be padded to multiple of 256 for copying image buffer
        // from/to GPU texture
        let padded_width = (256.0 * (dimensions.to_physical_width() as f32 / 256.0).ceil()) as u32;
        let padded_height =
            (256.0 * (dimensions.to_physical_height() as f32 / 256.0).ceil()) as u32;

        let output_buffer_size = (u32_size * padded_width * padded_height) as BufferAddress;
        let output_buffer_desc = BufferDescriptor {
            size: output_buffer_size,
            usage: BufferUsages::COPY_DST
                // this tells wpgu that we want to read this buffer from the cpu
                | BufferUsages::MAP_READ,
            label: None,
            mapped_at_creation: false,
        };
        let output_buffer = device.create_buffer(&output_buffer_desc);

        let multisampled_framebuffer = create_multisampled_framebuffer(
            &device,
            dimensions.to_physical_width(),
            dimensions.to_physical_height(),
            texture_format,
            sample_count,
        );

        let multi_render_resources =
            MultiMarkRenderResources::new(&device, texture_format, sample_count);

        let text_atlas_builder = make_text_atlas_builder(&config.text_builder_ctor);

        Ok(Self {
            device,
            queue,
            multisampled_framebuffer,
            sample_count,
            dimensions,
            texture,
            texture_view,
            output_buffer,
            texture_size,
            padded_width,
            padded_height,
            marks: Vec::new(),
            shared_multi: MultiMarkRenderer::new(dimensions),
            run_start: 0,
            current_zindex: 0,
            instanced_renderers: HashMap::new(),
            multi_render_resources,
            config,
            text_atlas_builder,
        })
    }

    fn commit_all_multi_renderers(&mut self) {
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(ZIndexedMark {
                zindex: self.current_zindex,
                renderer: MarkRenderer::Multi {
                    batch_range: self.run_start..end,
                },
            });
        }
        self.run_start = end;
    }

    #[tracing::instrument(skip_all)]
    pub async fn render(&mut self) -> Result<image::RgbaImage, AvengerWgpuError> {
        let render_start = Instant::now();
        self.commit_all_multi_renderers();

        // Build encoder for chart background
        let background_command = if self.sample_count > 1 {
            make_background_command(
                self,
                &self.multisampled_framebuffer,
                Some(&self.texture_view),
            )
        } else {
            make_background_command(self, &self.texture_view, None)
        };

        // Collect z-indices and compute layers
        let zindices: Vec<i32> = self.marks.iter().map(|m| m.zindex).collect();
        let layers = if zindices.is_empty() {
            vec![]
        } else {
            compute_zindex_layers(zindices)
        };
        let layer_count = layers.len();
        let (instanced_renderer_count, multi_renderer_count) = mark_renderer_counts(&self.marks);

        let mut commands = vec![background_command];
        let multi_render_resources = self.multi_render_resources.clone();
        let render_target_extent = Extent3d {
            width: self.dimensions.to_physical_width(),
            height: self.dimensions.to_physical_height(),
            depth_or_array_layers: 1,
        };

        // Build the shared text atlas bind groups ONCE per frame (instead of once per
        // multi-renderer). Every renderer this frame registered into the same atlas, so
        // these bind groups are page-correct for all of them.
        let (text_atlas_size, text_atlas_images) = self.text_atlas_builder.build();
        let text_bind_groups = MultiMarkRenderer::make_text_bind_groups_dual_sampler(
            &self.device,
            &self.queue,
            self.multi_render_resources.text_layout(),
            text_atlas_size,
            &text_atlas_images,
        );

        // Prepare the single shared multi-renderer ONCE per frame (uniform, gradient/
        // image atlas bind groups, stencil buffer, combined vertex/index/clip buffers).
        // Each z-run (a `MarkRenderer::Multi` batch range) is then encoded from this
        // shared prep, so the 40-ish per-cell renderers' setup collapses to one.
        let _prepare_start = Instant::now();
        let prepared = self.shared_multi.prepare(
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
        let command_build_start = Instant::now();
        // Coalesce consecutive multi-mark runs into one command buffer with merged
        // passes; instanced marks break a run, interleaved by (layer, document).
        let mut pending: Vec<std::ops::Range<usize>> = Vec::new();
        for (min_z, max_z) in layers {
            for mark in &self.marks {
                if mark.zindex >= min_z && mark.zindex <= max_z {
                    match &mark.renderer {
                        MarkRenderer::Multi { batch_range } => pending.push(batch_range.clone()),
                        MarkRenderer::Instanced {
                            renderer,
                            x_adjustment,
                            y_adjustment,
                        } => {
                            if !pending.is_empty() {
                                let c = if self.sample_count > 1 {
                                    self.shared_multi.encode_multi_ranges(
                                        &self.device,
                                        render_target_extent,
                                        &self.multisampled_framebuffer,
                                        Some(&self.texture_view),
                                        &multi_render_resources,
                                        &text_bind_groups,
                                        &prepared,
                                        &pending,
                                    )
                                } else {
                                    self.shared_multi.encode_multi_ranges(
                                        &self.device,
                                        render_target_extent,
                                        &self.texture_view,
                                        None,
                                        &multi_render_resources,
                                        &text_bind_groups,
                                        &prepared,
                                        &pending,
                                    )
                                };
                                commands.push(c);
                                pending.clear();
                            }
                            let c = if self.sample_count > 1 {
                                renderer.render(
                                    &self.device,
                                    &self.multisampled_framebuffer,
                                    Some(&self.texture_view),
                                    *x_adjustment,
                                    *y_adjustment,
                                )
                            } else {
                                renderer.render(
                                    &self.device,
                                    &self.texture_view,
                                    None,
                                    *x_adjustment,
                                    *y_adjustment,
                                )
                            };
                            commands.push(c);
                        }
                    }
                }
            }
        }
        if !pending.is_empty() {
            let c = if self.sample_count > 1 {
                self.shared_multi.encode_multi_ranges(
                    &self.device,
                    render_target_extent,
                    &self.multisampled_framebuffer,
                    Some(&self.texture_view),
                    &multi_render_resources,
                    &text_bind_groups,
                    &prepared,
                    &pending,
                )
            } else {
                self.shared_multi.encode_multi_ranges(
                    &self.device,
                    render_target_extent,
                    &self.texture_view,
                    None,
                    &multi_render_resources,
                    &text_bind_groups,
                    &prepared,
                    &pending,
                )
            };
            commands.push(c);
        }
        let command_build_elapsed = command_build_start.elapsed();
        let command_count = commands.len();

        let submit_start = Instant::now();
        self.queue.submit(commands);
        let submit_elapsed = submit_start.elapsed();

        // Extract texture from GPU
        let extract_start = Instant::now();
        let mut extract_encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Extract Texture Encoder"),
            });

        let u32_size = std::mem::size_of::<u32>() as u32;

        extract_encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                aspect: TextureAspect::All,
                texture: &self.texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
            },
            TexelCopyBufferInfo {
                buffer: &self.output_buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    // bytes_per_row: Some(u32_size * self.width as u32),
                    bytes_per_row: Some(u32_size * self.padded_width),
                    rows_per_image: Some(self.padded_height),
                },
            },
            self.texture_size,
        );
        self.queue.submit(Some(extract_encoder.finish()));
        let extract_elapsed = extract_start.elapsed();

        // Output to png file
        let map_read_start = Instant::now();
        let img = {
            let buffer_slice = self.output_buffer.slice(..);

            // NOTE: We have to create the mapping THEN device.poll() before await
            // the future. Otherwise the application will freeze.
            let (tx, rx) = futures_intrusive::channel::shared::oneshot_channel();
            buffer_slice.map_async(MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
            self.device.poll(wgpu::PollType::Wait).unwrap();

            // TODO: remove panic
            rx.receive().await.unwrap().unwrap();

            let data = buffer_slice.get_mapped_range();
            let img_buf =
                image::RgbaImage::from_vec(self.padded_width, self.padded_height, data.to_vec())
                    .unwrap();

            let cropped_img = crop_imm(
                &img_buf,
                0,
                0,
                self.dimensions.to_physical_width(),
                self.dimensions.to_physical_height(),
            );
            cropped_img.to_image()
        };
        let map_read_elapsed = map_read_start.elapsed();

        self.output_buffer.unmap();
        tracing::debug!(
            target: "avenger_wgpu::resize",
            render_ms = render_start.elapsed().as_secs_f64() * 1000.0,
            command_build_ms = command_build_elapsed.as_secs_f64() * 1000.0,
            submit_ms = submit_elapsed.as_secs_f64() * 1000.0,
            extract_ms = extract_elapsed.as_secs_f64() * 1000.0,
            map_read_ms = map_read_elapsed.as_secs_f64() * 1000.0,
            physical_width = self.dimensions.to_physical_width(),
            physical_height = self.dimensions.to_physical_height(),
            command_count,
            renderer_count = self.marks.len(),
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
        if zindex != self.current_zindex {
            self.commit_multi_renderer_if_needed(zindex);
        }
        self.current_zindex = zindex;
    }

    fn commit_multi_renderer_if_needed(&mut self, _new_zindex: i32) {
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(ZIndexedMark {
                zindex: self.current_zindex,
                renderer: MarkRenderer::Multi {
                    batch_range: self.run_start..end,
                },
            });
        }
        self.run_start = end;
    }

    fn get_current_zindex(&self) -> i32 {
        self.current_zindex
    }

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        &mut self.shared_multi
    }

    fn text_atlas_builder(&mut self) -> &mut dyn TextAtlasBuilderTrait {
        &mut *self.text_atlas_builder
    }

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.instanced_renderers.get(&fingerprint).cloned()
    }

    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(ZIndexedMark {
                zindex: self.current_zindex,
                renderer: MarkRenderer::Multi {
                    batch_range: self.run_start..end,
                },
            });
        }
        self.run_start = end;
        self.instanced_renderers
            .insert(fingerprint, mark_renderer.clone());
        self.marks.push(ZIndexedMark {
            zindex: self.current_zindex,
            renderer: MarkRenderer::Instanced {
                renderer: mark_renderer,
                x_adjustment,
                y_adjustment,
            },
        });
    }

    fn clear_mark_renderer(&mut self) {
        // One shared multi-renderer per canvas: reset it in place each frame and
        // clear the recorded z-run marks. (Instanced fingerprint cache is retained.)
        self.shared_multi.reset_for_frame(self.dimensions);
        self.run_start = 0;
        self.marks.clear();

        // Reset the shared text atlas so each frame starts clean (matches the old
        // per-renderer reset-per-frame semantics).
        self.text_atlas_builder = make_text_atlas_builder(&self.config.text_builder_ctor);
    }

    fn device(&self) -> &Device {
        &self.device
    }

    fn queue(&self) -> &Queue {
        &self.queue
    }

    fn dimensions(&self) -> CanvasDimensions {
        self.dimensions
    }

    fn texture_format(&self) -> TextureFormat {
        self.texture.format()
    }

    fn sample_count(&self) -> u32 {
        self.sample_count
    }
}
