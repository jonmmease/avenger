use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use avenger_common::{canvas::CanvasDimensions, time::Instant, types::LinearScaleAdjustment};
use avenger_eventstream::runtime::{RuntimeTooltipState, RuntimeTooltipUpdate};
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark,
        area::SceneAreaMark,
        group::Clip,
        image::SceneImageMark,
        line::SceneLineMark,
        mark::SceneMark,
        path::ScenePathMark,
        pattern::{is_no_fill_pattern, PatternReferenceFrame},
        rect::SceneRectMark,
        rule::SceneRuleMark,
        symbol::SceneSymbolMark,
        text::SceneTextMark,
        trail::SceneTrailMark,
    },
    pattern_geometry::PatternRect,
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use avenger_typst_label::LabelEngine;
use wgpu::{
    Adapter, CommandEncoderDescriptor, Device, DeviceDescriptor, Extent3d, PowerPreference, Queue,
    RequestAdapterError, RequestAdapterOptions, Surface, SurfaceConfiguration, TextureDescriptor,
    TextureDimension, TextureFormat, TextureFormatFeatureFlags, TextureUsages, TextureView,
    TextureViewDescriptor, Trace,
};
use winit::{
    dpi::{PhysicalSize, Size},
    event::WindowEvent,
    window::Window,
};

use crate::{
    error::AvengerWgpuError,
    image_resources::{WgpuImageResourceConfig, WgpuImageResourceStatus},
    marks::{
        instanced_mark::{InstancedMarkFingerprint, InstancedMarkRenderer},
        multi::{is_axis_aligned_angle, MultiMarkRenderer},
        symbol::SymbolShader,
        text::{TextAtlasBuilder, TextInstance},
    },
    offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
    readback::TextureReadback,
    renderer::{mark_renderer_counts, AvengerRendererCore},
    target::{AvengerRenderTarget, WHITE_CLEAR},
    tooltip::TooltipOverlayLayout,
};
use avenger_scenegraph::render_order::compute_zindex_layers;

pub use crate::renderer::{MarkRenderer, ZIndexedMark};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasFrameOverlay {
    pub size: [f32; 2],
    pub resize_width: bool,
    pub resize_height: bool,
    pub handle_thickness: f32,
}

pub trait CanvasDimensionUtils {
    fn to_physical_size(&self) -> winit::dpi::PhysicalSize<u32>;
}

fn instanced_symbol_renderer_cache_key(
    mark: &SceneSymbolMark,
    origin: [f32; 2],
    dimensions: CanvasDimensions,
    clip: &Clip,
) -> u64 {
    let mut hasher = DefaultHasher::new();

    // The mark fingerprint covers the geometry and per-instance data. The remaining
    // values are baked into immutable renderer uniforms and therefore must also match
    // before an existing renderer can be reused.
    "symbol".hash(&mut hasher);
    mark.instanced_fingerprint().hash(&mut hasher);
    origin.map(f32::to_bits).hash(&mut hasher);
    dimensions.size.map(f32::to_bits).hash(&mut hasher);
    dimensions.scale.to_bits().hash(&mut hasher);
    clip.hash(&mut hasher);

    hasher.finish()
}

impl CanvasDimensionUtils for CanvasDimensions {
    fn to_physical_size(&self) -> winit::dpi::PhysicalSize<u32> {
        winit::dpi::PhysicalSize {
            width: self.to_physical_width(),
            height: self.to_physical_height(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn window_accepted_or_requested_size(
    requested_size: PhysicalSize<u32>,
    _accepted_size: PhysicalSize<u32>,
) -> PhysicalSize<u32> {
    requested_size
}

#[cfg(not(target_arch = "wasm32"))]
fn window_accepted_or_requested_size(
    _requested_size: PhysicalSize<u32>,
    accepted_size: PhysicalSize<u32>,
) -> PhysicalSize<u32> {
    accepted_size
}

#[derive(Clone, Default)]
pub struct CanvasConfig {
    pub image_resource_config: WgpuImageResourceConfig,
    pub sample_count: Option<u32>,
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

    /// Start installing a scene, retaining it and the engine that draws its text for
    /// subsequent frames.
    fn begin_scene(&mut self, _scene: &SceneGraph, _text_engine: &LabelEngine) {
        self.clear_mark_renderer();
    }

    /// Mark a scene installation as complete.
    fn finish_scene(&mut self) {}
    fn device(&self) -> &Device;
    fn queue(&self) -> &Queue;
    fn dimensions(&self) -> CanvasDimensions;

    fn texture_format(&self) -> TextureFormat;

    fn sample_count(&self) -> u32;

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer;

    /// The text atlas shared by every multi-renderer on this canvas. Glyphs are
    /// registered into it during the set_scene mark walk, and it is built + uploaded
    /// once per frame at render time.
    fn text_atlas_builder(&mut self) -> &mut TextAtlasBuilder;

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>>;

    fn set_current_zindex(&mut self, zindex: i32);

    fn add_arc_mark(
        &mut self,
        mark: &SceneArcMark,
        origin: [f32; 2],
        group_clip: &Clip,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_arc_mark(
            mark,
            origin,
            group_clip,
            pattern_reference_frame,
            chart_bounds,
        )?;
        Ok(())
    }

    fn add_path_mark(
        &mut self,
        mark: &ScenePathMark,
        origin: [f32; 2],
        group_clip: &Clip,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_path_mark(
            mark,
            origin,
            group_clip,
            pattern_reference_frame,
            chart_bounds,
        )?;
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
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_area_mark(
            mark,
            origin,
            group_clip,
            pattern_reference_frame,
            chart_bounds,
        )?;
        Ok(())
    }

    fn add_symbol_mark(
        &mut self,
        mark: &SceneSymbolMark,
        origin: [f32; 2],
        group_clip: &Clip,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        if symbol_mark_is_instanced_eligible(mark, group_clip) {
            let effective_clip = group_clip.maybe_clip(mark.clip);
            let fingerprint = instanced_symbol_renderer_cache_key(
                mark,
                origin,
                self.dimensions(),
                &effective_clip,
            );
            let renderer = if let Some(renderer) = self.get_instanced_renderer(fingerprint) {
                tracing::debug!(target: "avenger_wgpu::retained_symbols", mark = %mark.name, fingerprint, renderer = ?Arc::as_ptr(&renderer), "reused instanced symbol renderer");
                renderer
            } else {
                let _span = tracing::debug_span!(target: "avenger_wgpu::retained_symbols", "build_symbol_renderer", mark = %mark.name, fingerprint).entered();
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
                    effective_clip,
                    self.dimensions().scale,
                ));
                tracing::debug!(target: "avenger_wgpu::retained_symbols", renderer = ?Arc::as_ptr(&renderer), "created instanced symbol renderer");
                renderer
            };

            self.add_instanced_mark_renderer(
                renderer,
                fingerprint,
                mark.x_adjustment,
                mark.y_adjustment,
            );
        } else {
            self.get_multi_renderer().add_symbol_mark(
                mark,
                origin,
                group_clip,
                pattern_reference_frame,
                chart_bounds,
            )?;
        }

        Ok(())
    }

    fn add_rect_mark(
        &mut self,
        mark: &SceneRectMark,
        origin: [f32; 2],
        group_clip: &Clip,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_rect_mark(
            mark,
            origin,
            group_clip,
            pattern_reference_frame,
            chart_bounds,
        )?;
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
        text_engine: &LabelEngine,
    ) -> Result<(), AvengerWgpuError> {
        let dimensions = self.dimensions();
        let text_atlas_builder = self.text_atlas_builder();
        let mut registrations = Vec::new();
        for label in mark.labels() {
            let instance = TextInstance {
                label: label.label,
                position: [label.position[0] + origin[0], label.position[1] + origin[1]],
                align: label.align,
                baseline: label.baseline,
                angle: label.angle,
                use_nearest_filter: is_axis_aligned_angle(label.angle),
            };
            registrations.extend(text_atlas_builder.register_text(
                instance,
                dimensions,
                text_engine,
            )?);
        }

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

    fn add_warped_image_mark(
        &mut self,
        mark: &avenger_scenegraph::marks::warped_image::SceneWarpedImageMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_warped_image_mark(mark, origin, group_clip)?;
        Ok(())
    }

    /// Installs a scene, drawing its text with the engine that measured it.
    #[tracing::instrument(skip_all)]
    fn set_scene(
        &mut self,
        scene_graph: &SceneGraph,
        text_engine: &LabelEngine,
    ) -> Result<(), AvengerWgpuError> {
        let start = Instant::now();
        let clear_start = Instant::now();
        self.begin_scene(scene_graph, text_engine);
        self.set_current_zindex(0);
        let clear_elapsed = clear_start.elapsed();

        // Process display items in document order. Z-index sorting happens during rendering.
        let chart_bounds = PatternRect::new(0.0, 0.0, scene_graph.width, scene_graph.height);
        let display_list_start = Instant::now();
        let display_list = SceneDisplayList::from_scene_graph(scene_graph);
        let display_list_elapsed = display_list_start.elapsed();
        let item_count = display_list.items.len();
        let mut image_ms = 0.0;
        let mut text_ms = 0.0;
        let mut other_ms = 0.0;
        let mut text_mark_count = 0u64;
        let mut text_instance_count = 0u64;
        for item in &display_list.items {
            self.set_current_zindex(item.zindex);
            let item_start = Instant::now();
            let mut item_kind = "other";
            match &item.mark {
                SceneDisplayMark::OwnedGroupPath(mark) => {
                    self.add_path_mark(
                        mark,
                        item.origin,
                        &item.clip,
                        item.pattern_reference_frame.as_ref(),
                        chart_bounds,
                    )?;
                }
                SceneDisplayMark::Borrowed(mark) => match mark {
                    SceneMark::Arc(mark) => {
                        self.add_arc_mark(
                            mark,
                            item.origin,
                            &item.clip,
                            item.pattern_reference_frame.as_ref(),
                            chart_bounds,
                        )?;
                    }
                    SceneMark::Symbol(mark) => {
                        self.add_symbol_mark(
                            mark,
                            item.origin,
                            &item.clip,
                            item.pattern_reference_frame.as_ref(),
                            chart_bounds,
                        )?;
                    }
                    SceneMark::Rect(mark) => {
                        self.add_rect_mark(
                            mark,
                            item.origin,
                            &item.clip,
                            item.pattern_reference_frame.as_ref(),
                            chart_bounds,
                        )?;
                    }
                    SceneMark::Rule(mark) => {
                        self.add_rule_mark(mark, item.origin, &item.clip)?;
                    }
                    SceneMark::Path(mark) => {
                        self.add_path_mark(
                            mark,
                            item.origin,
                            &item.clip,
                            item.pattern_reference_frame.as_ref(),
                            chart_bounds,
                        )?;
                    }
                    SceneMark::Line(mark) => {
                        self.add_line_mark(mark, item.origin, &item.clip)?;
                    }
                    SceneMark::Trail(mark) => {
                        self.add_trail_mark(mark, item.origin, &item.clip)?;
                    }
                    SceneMark::Area(mark) => {
                        self.add_area_mark(
                            mark,
                            item.origin,
                            &item.clip,
                            item.pattern_reference_frame.as_ref(),
                            chart_bounds,
                        )?;
                    }
                    SceneMark::Text(mark) => {
                        item_kind = "text";
                        text_mark_count += 1;
                        text_instance_count += u64::from(mark.len);
                        self.add_text_mark(mark, item.origin, &item.clip, text_engine)?;
                    }
                    SceneMark::Image(mark) => {
                        item_kind = "image";
                        self.add_image_mark(mark, item.origin, &item.clip)?;
                    }
                    SceneMark::WarpedImage(mark) => {
                        item_kind = "image";
                        self.add_warped_image_mark(mark, item.origin, &item.clip)?;
                    }
                    SceneMark::Group(_) => {}
                },
            }
            let elapsed_ms = item_start.elapsed().as_secs_f64() * 1000.0;
            match item_kind {
                "image" => image_ms += elapsed_ms,
                "text" => text_ms += elapsed_ms,
                _ => other_ms += elapsed_ms,
            }
        }
        self.set_current_zindex(0);

        tracing::debug!(
            target: "avenger_wgpu::resize",
            set_scene_ms = start.elapsed().as_secs_f64() * 1000.0,
            clear_ms = clear_elapsed.as_secs_f64() * 1000.0,
            display_list_ms = display_list_elapsed.as_secs_f64() * 1000.0,
            image_ms,
            text_ms,
            other_ms,
            text_mark_count,
            text_instance_count,
            item_count,
            scene_width = scene_graph.width,
            scene_height = scene_graph.height,
            "wgpu.set_scene"
        );
        self.finish_scene();
        Ok(())
    }
}

fn symbol_mark_is_instanced_eligible(mark: &SceneSymbolMark, group_clip: &Clip) -> bool {
    mark.len >= 100
        && mark.gradients.is_empty()
        && is_no_fill_pattern(&mark.fill_pattern)
        && matches!(group_clip, Clip::None | Clip::Rect { .. })
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
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
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

pub(crate) fn select_sample_count(
    sample_flags: TextureFormatFeatureFlags,
    requested: Option<u32>,
    default_sample_count: u32,
) -> u32 {
    let supported = get_supported_sample_count(sample_flags);
    let requested = requested.unwrap_or(default_sample_count).max(1);
    if requested >= 4 && supported >= 4 {
        4
    } else if requested >= 2 && supported >= 2 {
        2
    } else {
        1
    }
}

pub struct WindowCanvas<'window> {
    surface_config: SurfaceConfiguration,
    renderer: AvengerRendererCore,
    frame_overlay: Option<CanvasFrameOverlay>,
    tooltip_state: RuntimeTooltipState,
    tooltip_overlay: Option<TooltipOverlayLayout>,

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
        window: impl Into<Arc<Window>>,
        dimensions: CanvasDimensions,
        config: CanvasConfig,
    ) -> Result<Self, AvengerWgpuError> {
        let window = window.into();
        let requested_size = dimensions.to_physical_size();
        let accepted_size = window
            .request_inner_size(Size::Physical(requested_size))
            .map(|accepted| window_accepted_or_requested_size(requested_size, accepted))
            .unwrap_or_else(|| {
                window_accepted_or_requested_size(requested_size, window.inner_size())
            });
        let dimensions = CanvasDimensions {
            size: [
                accepted_size.width as f32 / dimensions.scale,
                accepted_size.height as f32 / dimensions.scale,
            ],
            scale: dimensions.scale,
        };
        let instance = make_wgpu_instance();
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

        let format_flags = adapter.get_texture_format_features(surface_format).flags;
        let sample_count = select_sample_count(
            format_flags,
            config.sample_count,
            get_supported_sample_count(format_flags),
        );
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
            tooltip_state: RuntimeTooltipState::default(),
            tooltip_overlay: None,
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

    /// Apply a host-neutral tooltip update and request a frame when it changes.
    pub fn set_tooltip_update(
        &mut self,
        update: RuntimeTooltipUpdate,
    ) -> Result<bool, AvengerWgpuError> {
        let changed = self.tooltip_state.apply(update.clone());
        if !changed {
            return Ok(false);
        }
        match update {
            RuntimeTooltipUpdate::Move { owner, anchor } => {
                if let Some(overlay) = self.tooltip_overlay.as_mut() {
                    if overlay.presentation.owner == owner {
                        overlay.presentation.anchor = anchor;
                    }
                }
            }
            RuntimeTooltipUpdate::Show(presentation) => {
                // A tooltip measures and draws with the engine of the scene it's over.
                self.tooltip_overlay = None;
                if let Some(text_engine) = self.renderer.text_engine() {
                    let layout = TooltipOverlayLayout::new(presentation, text_engine)
                        .map_err(avenger_typst_label::RasterError::Label)?;
                    self.tooltip_overlay = Some(layout);
                }
            }
            RuntimeTooltipUpdate::Hide { .. } | RuntimeTooltipUpdate::Clear => {
                self.tooltip_overlay = None;
            }
        }
        self.window.request_redraw();
        Ok(true)
    }

    pub fn clear_tooltip(&mut self) {
        let _ = self.set_tooltip_update(RuntimeTooltipUpdate::Clear);
    }

    pub fn image_resource_status(&self) -> &WgpuImageResourceStatus {
        self.renderer.image_resource_status()
    }

    pub fn set_image_resource_resolver(
        &mut self,
        resolver: Arc<dyn crate::image_resources::ImageResourceResolver>,
    ) {
        self.renderer.set_image_resource_resolver(resolver);
    }

    /// Tile texture-array upload accounting: `(most_recent_frame, cumulative)`.
    pub fn tile_upload_stats(
        &self,
    ) -> (
        crate::image_resources::TileUploadStats,
        crate::image_resources::TileUploadStats,
    ) {
        self.renderer.tile_upload_stats()
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

        self.renderer.commit_all_multi_renderers();
        let marks = self.renderer.marks().to_vec();

        let zindices: Vec<i32> = marks.iter().map(|m| m.zindex).collect();
        let layers = if zindices.is_empty() {
            vec![]
        } else {
            compute_zindex_layers(zindices)
        };
        let layer_count = layers.len();
        let (instanced_renderer_count, multi_renderer_count) = mark_renderer_counts(&marks);

        let command_build_start = Instant::now();
        let render_target = if sample_count > 1 {
            AvengerRenderTarget::multisampled(
                &self.multisampled_framebuffer,
                &view,
                render_target_extent,
                self.renderer.texture_format(),
                sample_count,
                WHITE_CLEAR,
            )
        } else {
            AvengerRenderTarget::new(
                &view,
                render_target_extent,
                self.renderer.texture_format(),
                1,
                WHITE_CLEAR,
            )
        };
        let commands = self.renderer.build_frame_commands(
            &self.device,
            &self.queue,
            render_target,
            self.frame_overlay,
            self.tooltip_overlay.as_ref(),
        )?;

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

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.renderer.shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut TextAtlasBuilder {
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

    fn begin_scene(
        &mut self,
        scene: &avenger_scenegraph::scene_graph::SceneGraph,
        text_engine: &LabelEngine,
    ) {
        self.renderer.begin_scene(scene, text_engine);
    }

    fn finish_scene(&mut self) {
        self.renderer.finish_scene();
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
        let sample_count = select_sample_count(
            format_flags,
            config.sample_count,
            get_supported_sample_count(format_flags),
        );
        let output_target_descriptor = OffscreenTargetDescriptor::new(dimensions, texture_format)
            .with_usage(TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT)
            .with_label("PngCanvas output texture");
        let output_target = OffscreenTarget::new(&device, &output_target_descriptor);
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
        self.render_with_tooltip_overlay(None).await
    }

    async fn render_with_tooltip_overlay(
        &mut self,
        tooltip: Option<&TooltipOverlayLayout>,
    ) -> Result<image::RgbaImage, AvengerWgpuError> {
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
        let commands = self.renderer.build_frame_commands(
            &self.device,
            &self.queue,
            render_target,
            None,
            tooltip,
        )?;
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

    pub fn image_resource_status(&self) -> &WgpuImageResourceStatus {
        self.renderer.image_resource_status()
    }

    /// Tile texture-array upload accounting: `(most_recent_frame, cumulative)`.
    pub fn tile_upload_stats(
        &self,
    ) -> (
        crate::image_resources::TileUploadStats,
        crate::image_resources::TileUploadStats,
    ) {
        self.renderer.tile_upload_stats()
    }
}

impl Canvas for PngCanvas {
    fn set_current_zindex(&mut self, zindex: i32) {
        self.renderer.set_current_zindex(zindex);
    }

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.renderer.shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut TextAtlasBuilder {
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

    fn begin_scene(
        &mut self,
        scene: &avenger_scenegraph::scene_graph::SceneGraph,
        text_engine: &LabelEngine,
    ) {
        self.renderer.begin_scene(scene, text_engine);
    }

    fn finish_scene(&mut self) {
        self.renderer.finish_scene();
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

#[cfg(test)]
mod tooltip_overlay_tests {
    use std::{fs, path::PathBuf};

    use avenger_eventstream::runtime::{
        RuntimeTooltipPresentation, RuntimeTooltipRow, RuntimeTooltipStyle,
    };

    use super::*;

    #[test]
    fn tooltip_overlay_offscreen_content_baseline() {
        let dimensions = CanvasDimensions {
            size: [480.0, 320.0],
            scale: 1.0,
        };
        let config = CanvasConfig::default();
        let text_engine = avenger_typst_label::bundled_label_engine();
        let style = RuntimeTooltipStyle {
            max_width: 300.0,
            ..RuntimeTooltipStyle::default()
        };
        let layout = TooltipOverlayLayout::new(
            RuntimeTooltipPresentation {
                owner: "baseline".into(),
                anchor: [470.0, 310.0],
                offset: [12.0, 12.0],
                rows: vec![
                    RuntimeTooltipRow {
                        label: "Short".into(),
                        value: "Falcon".into(),
                    },
                    RuntimeTooltipRow {
                        label: "Long label that truncates".into(),
                        value: "A long value wraps without escaping the canvas boundary".into(),
                    },
                    RuntimeTooltipRow {
                        label: "Unicode".into(),
                        value: "東京 · naïve · λ".into(),
                    },
                    RuntimeTooltipRow {
                        label: "Multiline".into(),
                        value: "first line\nsecond line".into(),
                    },
                    RuntimeTooltipRow {
                        label: "Null".into(),
                        value: "—".into(),
                    },
                    RuntimeTooltipRow {
                        label: "Nested".into(),
                        value: r#"{"a":[1,true],"b":{"c":"d"}}"#.into(),
                    },
                ],
                style,
            },
            &text_engine,
        )
        .expect("tooltip layout");
        let mut canvas = pollster::block_on(PngCanvas::new(dimensions, config))
            .expect("tooltip baseline canvas");
        let actual = pollster::block_on(canvas.render_with_tooltip_overlay(Some(&layout)))
            .expect("render tooltip baseline");

        let baseline = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/baselines/tooltip-overlay-content.png");
        if std::env::var_os("AVENGER_WGPU_UPDATE_BASELINES").is_some() {
            fs::create_dir_all(baseline.parent().expect("baseline parent"))
                .expect("create tooltip baseline directory");
            actual.save(&baseline).expect("save tooltip baseline");
            eprintln!("updated {}", baseline.display());
            return;
        }

        let expected = image::open(&baseline)
            .unwrap_or_else(|error| panic!("load {}: {error}", baseline.display()))
            .into_rgba8();
        assert_eq!(actual.dimensions(), expected.dimensions());
        let total_difference = actual
            .pixels()
            .zip(expected.pixels())
            .flat_map(|(actual, expected)| {
                actual
                    .0
                    .into_iter()
                    .zip(expected.0)
                    .map(|(actual, expected)| actual.abs_diff(expected) as u64)
            })
            .sum::<u64>();
        let normalized_difference = total_difference as f64
            / (actual.width() as f64 * actual.height() as f64 * 4.0 * 255.0);
        assert!(
            normalized_difference < 0.01,
            "tooltip baseline drifted by {normalized_difference:.6}"
        );
        assert!(
            actual
                .pixels()
                .filter(|pixel| pixel.0[..3] != [255, 255, 255])
                .count()
                > 1_000,
            "tooltip overlay should render visible content"
        );
    }
}

#[cfg(test)]
mod retained_symbol_tests {
    use super::*;
    use avenger_common::value::ScalarOrArray;
    use avenger_scenegraph::marks::group::SceneGroup;

    #[test]
    fn adjustments_reuse_buffers_and_match_fresh_positions() {
        let dimensions = CanvasDimensions {
            size: [128., 128.],
            scale: 1.,
        };
        let clip = Clip::Rect {
            x: 0.,
            y: 0.,
            width: 128.,
            height: 128.,
        };
        let base = SceneSymbolMark {
            len: 256,
            x: ScalarOrArray::new_array((0..256).map(|i| 4. + (i % 16) as f32 * 7.).collect()),
            y: ScalarOrArray::new_array((0..256).map(|i| 4. + (i / 16) as f32 * 7.).collect()),
            size: ScalarOrArray::new_scalar(9.),
            clip: true,
            interactive: false,
            ..Default::default()
        };
        let scene = |mark| SceneGraph {
            width: 128.,
            height: 128.,
            origin: [0., 0.],
            marks: vec![SceneMark::Group(SceneGroup {
                clip: clip.clone(),
                marks: vec![SceneMark::Symbol(mark)],
                ..Default::default()
            })],
        };
        let mut canvas =
            pollster::block_on(PngCanvas::new(dimensions, CanvasConfig::default())).unwrap();
        canvas.set_scene(&scene(base.clone())).unwrap();
        let key = instanced_symbol_renderer_cache_key(&base, [0., 0.], dimensions, &clip);
        let first = canvas.get_instanced_renderer(key).unwrap();
        pollster::block_on(canvas.render()).unwrap();
        let mut adjusted = base;
        adjusted.x_adjustment = Some(LinearScaleAdjustment {
            scale: 1.25,
            offset: -11.,
        });
        adjusted.y_adjustment = Some(LinearScaleAdjustment {
            scale: 0.75,
            offset: 7.,
        });
        canvas.set_scene(&scene(adjusted.clone())).unwrap();
        let second = canvas.get_instanced_renderer(key).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let actual = pollster::block_on(canvas.render()).unwrap();
        let mut fresh = adjusted.clone();
        fresh.x = ScalarOrArray::new_array(adjusted.x_vec());
        fresh.y = ScalarOrArray::new_array(adjusted.y_vec());
        fresh.x_adjustment = None;
        fresh.y_adjustment = None;
        canvas.set_scene(&scene(fresh)).unwrap();
        let expected = pollster::block_on(canvas.render()).unwrap();
        assert_eq!(actual, expected);

        // Viewport and scale-factor changes legitimately invalidate the cached renderer.
        for dimensions in [
            CanvasDimensions {
                size: [160., 128.],
                scale: 1.,
            },
            CanvasDimensions {
                size: [128., 128.],
                scale: 2.,
            },
        ] {
            canvas.renderer.set_dimensions(dimensions);
            canvas.set_scene(&scene(adjusted.clone())).unwrap();
            let changed_key =
                instanced_symbol_renderer_cache_key(&adjusted, [0., 0.], dimensions, &clip);
            assert_ne!(changed_key, key);
            let changed = canvas.get_instanced_renderer(changed_key).unwrap();
            assert!(!Arc::ptr_eq(&first, &changed));
        }
    }
}
