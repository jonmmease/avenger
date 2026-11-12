//! Canvases that draw an app's scenes into HTML canvas elements through the browser's WebGPU. The
//! canvases of a page share one device, so their pipelines compile once.

use std::sync::Arc;

use avenger_common::{canvas::CanvasDimensions, types::LinearScaleAdjustment};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;
use web_sys::HtmlCanvasElement;
use wgpu::{
    Adapter, Device, Instance, Queue, Surface, SurfaceConfiguration, SurfaceTarget, TextureUsages,
    TextureView, TextureViewDescriptor,
};

use crate::{
    canvas::{
        create_multisampled_framebuffer, get_supported_sample_count, make_wgpu_adapter,
        request_wgpu_device, select_sample_count, Canvas, CanvasConfig, ImageAnswers, Layers,
    },
    context::AvengerGpuContext,
    error::AvengerWgpuError,
    marks::{
        instanced_mark::{InstancedMarkPipeline, InstancedMarkRenderer},
        multi::MultiMarkRenderer,
        text::TextAtlasBuilder,
    },
    renderer::ImageStatus,
    target::surface_frame_target,
};

/// The browser's WebGPU device and the pipelines of its renderers, which the canvases of a page
/// share. A clone shares the same device.
#[derive(Clone)]
pub struct BrowserGpu {
    instance: Instance,
    adapter: Adapter,
    device: Device,
    queue: Queue,
    context: AvengerGpuContext,
}

impl BrowserGpu {
    /// Opens the browser's WebGPU device. A browser without WebGPU gives
    /// `AvengerWgpuError::WebGpuUnavailable`.
    pub async fn new() -> Result<Self, AvengerWgpuError> {
        if !wgpu::util::is_browser_webgpu_supported().await {
            return Err(AvengerWgpuError::WebGpuUnavailable);
        }
        let instance = Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });
        let adapter = make_wgpu_adapter(&instance, None).await?;
        let (device, queue) = request_wgpu_device(&adapter).await?;
        let context = AvengerGpuContext::new(&device);
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            context,
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// The longest side of a canvas that the device draws, in physical pixels.
    pub fn max_texture_dimension(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }
}

/// A canvas element that draws scenes in layers, from the bottom, on a `BrowserGpu`.
pub struct HtmlCanvasCanvas {
    surface_config: SurfaceConfiguration,
    layers: Layers,
    canvas: HtmlCanvasElement,

    // The order of properties determines the drop order, and the device must be dropped after
    // the buffers and textures of the marks.
    multisampled_framebuffer: TextureView,
    surface: Surface<'static>,
    queue: Queue,
    device: Device,
}

impl HtmlCanvasCanvas {
    /// A canvas that draws into `canvas` at `dimensions`, whose physical size it gives the
    /// element's drawing buffer.
    pub fn new(
        gpu: &BrowserGpu,
        canvas: HtmlCanvasElement,
        dimensions: CanvasDimensions,
        config: CanvasConfig,
    ) -> Result<Self, AvengerWgpuError> {
        let [width, height] = physical(dimensions);
        canvas.set_width(width);
        canvas.set_height(height);
        let surface = gpu
            .instance
            .create_surface(SurfaceTarget::Canvas(canvas.clone()))?;
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = (caps.formats.iter().copied())
            .find(|format| !format.is_srgb())
            .unwrap_or(caps.formats[0]);
        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&gpu.device, &surface_config);

        let flags = gpu.adapter.get_texture_format_features(format).flags;
        let sample_count = select_sample_count(
            flags,
            config.sample_count,
            get_supported_sample_count(flags),
        );
        let multisampled_framebuffer =
            create_multisampled_framebuffer(&gpu.device, width, height, format, sample_count);
        let layers = Layers::new(
            gpu.context.clone(),
            dimensions,
            format,
            sample_count,
            config.images,
        );
        Ok(Self {
            surface_config,
            layers,
            canvas,
            multisampled_framebuffer,
            surface,
            queue: gpu.queue.clone(),
            device: gpu.device.clone(),
        })
    }

    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
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

    /// The installed layers' image resources that are pending or failed, as of the last frame.
    pub fn image_status(&self) -> ImageStatus {
        self.layers.image_status()
    }

    /// Draws at `dimensions` from the next frame, and gives the element's drawing buffer their
    /// physical size. The installed scenes draw again at the new size.
    pub fn resize(&mut self, dimensions: CanvasDimensions) {
        let [width, height] = physical(dimensions);
        self.canvas.set_width(width);
        self.canvas.set_height(height);
        self.layers.set_dimensions(dimensions);
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        self.multisampled_framebuffer = create_multisampled_framebuffer(
            &self.device,
            width,
            height,
            self.surface_config.format,
            self.layers.bottom().sample_count(),
        );
    }

    /// Draws the layers into the canvas.
    pub fn render(&mut self) -> Result<(), AvengerWgpuError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());
        let target = surface_frame_target(
            &view,
            output.texture.size(),
            &self.multisampled_framebuffer,
            [self.surface_config.width, self.surface_config.height],
            self.layers.bottom().texture_format(),
            self.layers.bottom().sample_count(),
        );
        let commands = (self.layers).frame_commands(&self.device, &self.queue, target, None)?;
        self.queue.submit(commands);
        output.present();
        Ok(())
    }
}

/// The physical pixels of `dimensions`, at least one in each direction.
fn physical(dimensions: CanvasDimensions) -> [u32; 2] {
    [
        dimensions.to_physical_width().max(1),
        dimensions.to_physical_height().max(1),
    ]
}

impl Canvas for HtmlCanvasCanvas {
    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.layers.bottom_mut().shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut TextAtlasBuilder {
        self.layers.bottom_mut().text_atlas_builder_mut()
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

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.layers.bottom().get_instanced_renderer(fingerprint)
    }

    fn clear_mark_renderer(&mut self) {
        self.layers.bottom_mut().clear_mark_renderer();
    }

    fn begin_scene(&mut self, scene: &SceneGraph, text_engine: &LabelEngine) -> Arc<ImageAnswers> {
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
