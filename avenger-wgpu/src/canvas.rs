use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use avenger_common::{canvas::CanvasDimensions, types::LinearScaleAdjustment};
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark, area::SceneAreaMark, group::Clip, image::SceneImageMark,
        line::SceneLineMark, mark::SceneMark, path::ScenePathMark, rect::SceneRectMark,
        rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark, trail::SceneTrailMark,
    },
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
    context::AvengerGpuContext,
    error::AvengerWgpuError,
    image_resources::{WgpuImageResourceConfig, WgpuImageResourceStatus},
    marks::{
        instanced_mark::{InstancedMarkFingerprint, InstancedMarkPipeline, InstancedMarkRenderer},
        multi::{is_axis_aligned_angle, MultiMarkRenderer},
        symbol::SymbolShader,
        text::{TextAtlasBuilder, TextInstance},
    },
    offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
    readback::TextureReadback,
    renderer::AvengerRendererCore,
    target::{surface_frame_target, WHITE_CLEAR},
};

pub use crate::renderer::MarkRenderer;

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

/// Where a mark draws in its scene: what it takes from its enclosing groups and the scene.
#[derive(Debug, Clone, Copy)]
pub struct MarkContext<'a> {
    /// The origin of the mark's coordinates, in scene coordinates.
    pub origin: [f32; 2],
    /// The clip of the mark's enclosing groups.
    pub clip: &'a Clip,
    /// Where group-anchored patterns start: the pattern origin of the nearest enclosing group
    /// that sets one, or the scene's top-left corner.
    pub pattern_origin: [f32; 2],
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

    /// The pipeline that the canvas's instanced symbol marks share.
    fn symbol_pipeline(&self) -> Arc<InstancedMarkPipeline>;

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer;

    /// The text atlas shared by every multi-renderer on this canvas. Glyphs are
    /// registered into it during the set_scene mark walk, and it is built + uploaded
    /// once per frame at render time.
    fn text_atlas_builder(&mut self) -> &mut TextAtlasBuilder;

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>>;

    fn add_arc_mark(
        &mut self,
        mark: &SceneArcMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_arc_mark(mark, context)?;
        Ok(())
    }

    fn add_path_mark(
        &mut self,
        mark: &ScenePathMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_path_mark(mark, context)?;
        Ok(())
    }

    fn add_line_mark(
        &mut self,
        mark: &SceneLineMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_line_mark(mark, context.origin, context.clip)?;
        Ok(())
    }

    fn add_trail_mark(
        &mut self,
        mark: &SceneTrailMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_trail_mark(mark, context.origin, context.clip)?;
        Ok(())
    }

    fn add_area_mark(
        &mut self,
        mark: &SceneAreaMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_area_mark(mark, context)?;
        Ok(())
    }

    fn add_symbol_mark(
        &mut self,
        mark: &SceneSymbolMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        let (origin, group_clip) = (context.origin, context.clip);
        if symbol_mark_is_instanced_eligible(mark, group_clip) {
            let effective_clip = group_clip.maybe_clip(mark.clip);
            let fingerprint = instanced_symbol_renderer_cache_key(
                mark,
                origin,
                self.dimensions(),
                &effective_clip,
            );
            let renderer = if let Some(renderer) = self.get_instanced_renderer(fingerprint) {
                renderer
            } else {
                let shader = Box::new(SymbolShader::from_symbol_mark(
                    mark,
                    self.dimensions(),
                    origin,
                )?);

                let renderer = Arc::new(InstancedMarkRenderer::new(
                    self.device(),
                    &self.symbol_pipeline(),
                    shader,
                    effective_clip,
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
            self.get_multi_renderer().add_symbol_mark(mark, context)?;
        }

        Ok(())
    }

    fn add_rect_mark(
        &mut self,
        mark: &SceneRectMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_rect_mark(mark, context)?;
        Ok(())
    }

    fn add_rule_mark(
        &mut self,
        mark: &SceneRuleMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_rule_mark(mark, context.origin, context.clip)?;
        Ok(())
    }

    fn add_text_mark(
        &mut self,
        mark: &SceneTextMark,
        context: &MarkContext,
        text_engine: &LabelEngine,
    ) -> Result<(), AvengerWgpuError> {
        let (origin, group_clip) = (context.origin, context.clip);
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
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_image_mark(mark, context.origin, context.clip)?;
        Ok(())
    }

    fn add_warped_image_mark(
        &mut self,
        mark: &avenger_scenegraph::marks::warped_image::SceneWarpedImageMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_warped_image_mark(mark, context.origin, context.clip)?;
        Ok(())
    }

    /// Installs a scene, drawing its text with the engine that measured it.
    #[tracing::instrument(skip_all)]
    fn set_scene(
        &mut self,
        scene_graph: &SceneGraph,
        text_engine: &LabelEngine,
    ) -> Result<(), AvengerWgpuError> {
        self.begin_scene(scene_graph, text_engine);
        let display_list = SceneDisplayList::from_scene_graph(scene_graph);
        for item in display_list.ordered_items() {
            let context = MarkContext {
                origin: item.origin,
                clip: &item.clip,
                pattern_origin: item.pattern_origin,
            };
            match &item.mark {
                SceneDisplayMark::OwnedGroupPath(mark) => self.add_path_mark(mark, &context)?,
                SceneDisplayMark::Borrowed(mark) => match mark {
                    SceneMark::Arc(mark) => self.add_arc_mark(mark, &context)?,
                    SceneMark::Symbol(mark) => self.add_symbol_mark(mark, &context)?,
                    SceneMark::Rect(mark) => self.add_rect_mark(mark, &context)?,
                    SceneMark::Rule(mark) => self.add_rule_mark(mark, &context)?,
                    SceneMark::Path(mark) => self.add_path_mark(mark, &context)?,
                    SceneMark::Line(mark) => self.add_line_mark(mark, &context)?,
                    SceneMark::Trail(mark) => self.add_trail_mark(mark, &context)?,
                    SceneMark::Area(mark) => self.add_area_mark(mark, &context)?,
                    SceneMark::Text(mark) => self.add_text_mark(mark, &context, text_engine)?,
                    SceneMark::Image(mark) => self.add_image_mark(mark, &context)?,
                    SceneMark::WarpedImage(mark) => self.add_warped_image_mark(mark, &context)?,
                    SceneMark::Group(_) => {}
                },
            }
        }
        self.finish_scene();
        Ok(())
    }
}

fn symbol_mark_is_instanced_eligible(mark: &SceneSymbolMark, group_clip: &Clip) -> bool {
    mark.len >= 100
        && mark.gradients.is_empty()
        && mark.fill_pattern.is_none()
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

        let renderer = AvengerRendererCore::new(
            &AvengerGpuContext::new(&device),
            dimensions,
            surface_format,
            sample_count,
            config,
        );

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
        crate::marks::tile_array::TileUploadStats,
        crate::marks::tile_array::TileUploadStats,
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
        let output = self.surface.get_current_texture()?;
        let output_extent = output.texture.size();
        self.sync_to_acquired_surface_texture(output_extent);
        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());
        let target = surface_frame_target(
            &view,
            output_extent,
            &self.multisampled_framebuffer,
            [self.surface_config.width, self.surface_config.height],
            self.renderer.texture_format(),
            self.renderer.sample_count(),
        );
        let commands = self.renderer.build_frame_commands(
            &self.device,
            &self.queue,
            target,
            self.frame_overlay,
        )?;
        self.queue.submit(commands);
        output.present();
        Ok(())
    }
}

impl Canvas for WindowCanvas<'_> {
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

    fn symbol_pipeline(&self) -> Arc<InstancedMarkPipeline> {
        self.renderer.symbol_pipeline()
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
            .with_sample_count(sample_count)
            .with_label("PngCanvas output texture");
        let output_target = OffscreenTarget::new(&device, &output_target_descriptor);
        let readback = TextureReadback::new(&device, output_target.extent);

        let renderer = AvengerRendererCore::new(
            &AvengerGpuContext::new(&device),
            dimensions,
            texture_format,
            sample_count,
            config,
        );

        Ok(Self {
            device,
            queue,
            renderer,
            output_target,
            readback,
        })
    }

    /// The samples per pixel that the canvas draws with.
    pub fn sample_count(&self) -> u32 {
        self.renderer.sample_count()
    }

    #[tracing::instrument(skip_all)]
    pub async fn render(&mut self) -> Result<image::RgbaImage, AvengerWgpuError> {
        let dimensions = self.renderer.dimensions();
        let target = self.output_target.render_target(WHITE_CLEAR);
        let commands =
            self.renderer
                .build_frame_commands(&self.device, &self.queue, target, None)?;
        self.queue.submit(commands);

        let mut extract_encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Extract Texture Encoder"),
            });
        self.readback
            .encode_copy_from_texture(&mut extract_encoder, &self.output_target.texture);
        self.queue.submit(Some(extract_encoder.finish()));
        self.readback
            .read_rgba8(
                &self.device,
                dimensions.to_physical_width(),
                dimensions.to_physical_height(),
            )
            .await
    }

    pub fn image_resource_status(&self) -> &WgpuImageResourceStatus {
        self.renderer.image_resource_status()
    }

    /// Tile texture-array upload accounting: `(most_recent_frame, cumulative)`.
    pub fn tile_upload_stats(
        &self,
    ) -> (
        crate::marks::tile_array::TileUploadStats,
        crate::marks::tile_array::TileUploadStats,
    ) {
        self.renderer.tile_upload_stats()
    }
}

impl Canvas for PngCanvas {
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

    fn symbol_pipeline(&self) -> Arc<InstancedMarkPipeline> {
        self.renderer.symbol_pipeline()
    }
}
