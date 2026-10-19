use std::sync::Arc;

use avenger_common::canvas::CanvasDimensions;
use web_sys::HtmlCanvasElement;
use wgpu::{
    Device, Queue, Surface, SurfaceConfiguration, SurfaceTarget, TextureUsages, TextureView,
    TextureViewDescriptor,
};

use crate::{
    canvas::{
        create_multisampled_framebuffer, get_supported_sample_count, make_wgpu_adapter,
        request_wgpu_device, select_sample_count, Canvas, CanvasConfig, CanvasDimensionUtils,
    },
    context::AvengerGpuContext,
    error::AvengerWgpuError,
    marks::{
        instanced_mark::{InstancedMarkPipeline, InstancedMarkRenderer},
        multi::MultiMarkRenderer,
        text::TextAtlasBuilder,
    },
    renderer::AvengerRendererCore,
    target::surface_frame_target,
};

pub struct HtmlCanvasCanvas<'window> {
    surface_config: SurfaceConfiguration,
    renderer: AvengerRendererCore,

    // The order of properties determines that drop order and device must be dropped after
    // the buffers and textures associated with marks.
    multisampled_framebuffer: TextureView,
    queue: Queue,
    device: Device,
    surface: Surface<'window>,
}

impl<'window> HtmlCanvasCanvas<'window> {
    pub async fn new(
        canvas: HtmlCanvasElement,
        dimensions: CanvasDimensions,
        config: CanvasConfig,
    ) -> Result<Self, AvengerWgpuError> {
        canvas.set_width(dimensions.to_physical_width());
        canvas.set_height(dimensions.to_physical_height());
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::GL,
            ..Default::default()
        });
        let surface = instance.create_surface(SurfaceTarget::Canvas(canvas))?;
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

        let renderer = AvengerRendererCore::new(
            &AvengerGpuContext::new(&device),
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
            renderer,
        })
    }

    pub fn get_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.renderer.dimensions().to_physical_size()
    }

    pub fn resize(&mut self, _new_size: winit::dpi::PhysicalSize<u32>) {
        // if new_size.width > 0 && new_size.height > 0 {
        //     self.size = new_size;
        //     self.config.width = new_size.width;
        //     self.config.height = new_size.height;
        //     self.surface.configure(&self.device, &self.config);
        // }
    }

    pub fn render(&mut self) -> Result<(), AvengerWgpuError> {
        let output = self.surface.get_current_texture()?;
        let output_extent = output.texture.size();
        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());

        let render_target = surface_frame_target(
            &view,
            output_extent,
            &self.multisampled_framebuffer,
            [self.surface_config.width, self.surface_config.height],
            self.renderer.texture_format(),
            self.renderer.sample_count(),
        );

        let commands =
            self.renderer
                .build_frame_commands(&self.device, &self.queue, render_target, None)?;
        self.queue.submit(commands);
        output.present();

        Ok(())
    }
}

impl<'window> Canvas for HtmlCanvasCanvas<'window> {
    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.renderer.shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut TextAtlasBuilder {
        self.renderer.text_atlas_builder_mut()
    }

    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<avenger_common::types::LinearScaleAdjustment>,
        y_adjustment: Option<avenger_common::types::LinearScaleAdjustment>,
    ) {
        self.renderer.add_instanced_mark_renderer(
            mark_renderer,
            fingerprint,
            x_adjustment,
            y_adjustment,
        );
    }

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.renderer.get_instanced_renderer(fingerprint)
    }

    fn clear_mark_renderer(&mut self) {
        self.renderer.clear_mark_renderer();
    }

    fn begin_scene(
        &mut self,
        scene: &avenger_scenegraph::scene_graph::SceneGraph,
        text_engine: &avenger_typst_label::LabelEngine,
    ) -> Arc<crate::canvas::ImageAnswers> {
        self.renderer.begin_scene(scene, text_engine)
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
