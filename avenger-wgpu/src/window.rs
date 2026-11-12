//! A canvas that draws an app's scenes into a winit window.

use std::sync::Arc;

use avenger_common::{canvas::CanvasDimensions, types::LinearScaleAdjustment};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;
use wgpu::{
    Device, Extent3d, Queue, Surface, SurfaceConfiguration, TextureUsages, TextureView,
    TextureViewDescriptor,
};
use winit::{
    dpi::{PhysicalSize, Size},
    event::WindowEvent,
    window::Window,
};

use crate::{
    canvas::{
        create_multisampled_framebuffer, get_supported_sample_count, make_wgpu_adapter,
        make_wgpu_instance, request_wgpu_device, select_sample_count, Canvas, CanvasConfig,
        CanvasResizeFrame, ImageAnswers, Layers,
    },
    context::AvengerGpuContext,
    error::AvengerWgpuError,
    marks::{
        instanced_mark::{InstancedMarkPipeline, InstancedMarkRenderer},
        multi::MultiMarkRenderer,
        text::TextAtlasBuilder,
    },
    renderer::{FrameStats, ImageStatus},
    target::surface_frame_target,
};

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

pub struct WindowCanvas<'window> {
    surface_config: SurfaceConfiguration,
    layers: Layers,
    resize_frame: Option<CanvasResizeFrame>,

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

        let layers = Layers::new(
            AvengerGpuContext::new(&device),
            dimensions,
            surface_format,
            sample_count,
            config.images,
        );

        Ok(Self {
            surface,
            device,
            queue,
            multisampled_framebuffer,
            surface_config,
            window,
            layers,
            resize_frame: None,
        })
    }

    pub fn get_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.layers.bottom().dimensions().to_physical_size()
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Install `scene` at `index` without reinstalling other layers. Index 0 is the bottom layer.
    /// Missing intermediate layers are empty.
    pub fn set_layer(
        &mut self,
        index: usize,
        scene: &SceneGraph,
        text_engine: &LabelEngine,
    ) -> Result<(), AvengerWgpuError> {
        (self.layers).set(&self.device, &self.queue, index, scene, text_engine)
    }

    /// Set or clear the resize outline and handles drawn above the layers.
    pub fn set_resize_frame(&mut self, resize_frame: Option<CanvasResizeFrame>) {
        self.resize_frame = resize_frame;
    }

    /// The installed layers' image resources that are pending or failed, as of the last frame.
    pub fn image_status(&self) -> ImageStatus {
        self.layers.image_status()
    }

    /// Statistics for the bottom layer's most recent frame.
    pub fn frame_stats(&self) -> FrameStats {
        self.layers.bottom().frame_stats()
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.update_physical_size(new_size.width, new_size.height);
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    fn update_physical_size(&mut self, width: u32, height: u32) {
        let scale = self.layers.bottom().dimensions().scale;
        self.layers.set_dimensions(CanvasDimensions {
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
            self.layers.bottom().sample_count(),
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
            self.layers.bottom().texture_format(),
            self.layers.bottom().sample_count(),
        );
        let commands =
            self.layers
                .frame_commands(&self.device, &self.queue, target, self.resize_frame)?;
        self.queue.submit(commands);
        output.present();
        Ok(())
    }
}

impl Canvas for WindowCanvas<'_> {
    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.layers.bottom_mut().shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut TextAtlasBuilder {
        self.layers.bottom_mut().text_atlas_builder_mut()
    }

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.layers.bottom().get_instanced_renderer(fingerprint)
    }

    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        self.layers.bottom_mut().add_instanced_mark_renderer(
            mark_renderer,
            fingerprint,
            x_adjustment,
            y_adjustment,
        );
    }

    fn clear_mark_renderer(&mut self) {
        self.layers.bottom_mut().clear_mark_renderer();
    }

    fn begin_scene(
        &mut self,
        scene: &avenger_scenegraph::scene_graph::SceneGraph,
        text_engine: &LabelEngine,
    ) -> Arc<ImageAnswers> {
        self.layers.keep_bottom();
        self.layers.bottom_mut().begin_scene(scene, text_engine)
    }

    fn finish_scene(&mut self) {
        self.layers.bottom_mut().finish_scene();
    }

    fn device(&self) -> &Device {
        &self.device
    }

    fn queue(&self) -> &Queue {
        &self.queue
    }

    fn dimensions(&self) -> CanvasDimensions {
        self.layers.bottom().dimensions()
    }

    fn symbol_pipeline(&self) -> Arc<InstancedMarkPipeline> {
        self.layers.bottom().symbol_pipeline()
    }
}
