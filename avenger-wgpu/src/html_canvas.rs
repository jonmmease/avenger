use crate::canvas::{
    create_multisampled_framebuffer, get_supported_sample_count, make_background_command,
    make_wgpu_adapter, make_wgpu_instance, request_wgpu_device, Canvas, CanvasDimensions,
    MarkRenderer,
};
use crate::error::AvengerWgpuError;
use crate::marks::multi::MultiMarkRenderer;
use web_sys::HtmlCanvasElement;
use wgpu::{
    Device, Queue, Surface, SurfaceConfiguration, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};

pub struct HtmlCanvasCanvas {
    surface: Surface,
    device: Device,
    queue: Queue,
    multisampled_framebuffer: TextureView,
    sample_count: u32,
    config: SurfaceConfiguration,
    dimensions: CanvasDimensions,
    marks: Vec<MarkRenderer>,
    multi_renderer: Option<MultiMarkRenderer>,
}

impl HtmlCanvasCanvas {
    pub async fn new(
        canvas: HtmlCanvasElement,
        dimensions: CanvasDimensions,
    ) -> Result<Self, AvengerWgpuError> {
        canvas.set_width(dimensions.to_physical_width());
        canvas.set_height(dimensions.to_physical_height());
        let instance = make_wgpu_instance();
        let surface = instance.create_surface_from_canvas(canvas)?;
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

        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: dimensions.to_physical_width(),
            height: dimensions.to_physical_height(),
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let format_flags = adapter.get_texture_format_features(surface_format).flags;
        let sample_count = get_supported_sample_count(format_flags);
        let multisampled_framebuffer = create_multisampled_framebuffer(
            &device,
            config.width,
            config.height,
            surface_format,
            sample_count,
        );

        Ok(Self {
            surface,
            device,
            queue,
            multisampled_framebuffer,
            sample_count,
            config,
            dimensions,
            marks: Vec::new(),
            multi_renderer: None,
        })
    }

    pub fn get_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.dimensions.to_physical_size()
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
        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());

        // Commit open multi-renderer
        if let Some(multi_renderer) = self.multi_renderer.take() {
            self.marks
                .push(MarkRenderer::Multi(Box::new(multi_renderer)));
        }

        let background_command = if self.sample_count > 1 {
            make_background_command(self, &self.multisampled_framebuffer, Some(&view))
        } else {
            make_background_command(self, &view, None)
        };
        let mut commands = vec![background_command];
        let texture_format = self.texture_format();
        for mark in &mut self.marks {
            let command = match mark {
                MarkRenderer::Instanced(renderer) => {
                    if self.sample_count > 1 {
                        renderer.render(&self.device, &self.multisampled_framebuffer, Some(&view))
                    } else {
                        renderer.render(&self.device, &view, None)
                    }
                }
                MarkRenderer::Multi(renderer) => {
                    if self.sample_count > 1 {
                        renderer.render(
                            &self.device,
                            &self.queue,
                            texture_format,
                            self.sample_count,
                            &self.multisampled_framebuffer,
                            Some(&view),
                        )
                    } else {
                        renderer.render(
                            &self.device,
                            &self.queue,
                            texture_format,
                            self.sample_count,
                            &view,
                            None,
                        )
                    }
                }
            };

            commands.push(command);
        }

        self.queue.submit(commands);
        output.present();

        Ok(())
    }
}

impl Canvas for HtmlCanvasCanvas {
    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        if self.multi_renderer.is_none() {
            self.multi_renderer = Some(MultiMarkRenderer::new(self.dimensions));
        }
        self.multi_renderer.as_mut().unwrap()
    }

    fn add_mark_renderer(&mut self, mark_renderer: MarkRenderer) {
        if let Some(multi_renderer) = self.multi_renderer.take() {
            self.marks
                .push(MarkRenderer::Multi(Box::new(multi_renderer)));
        }
        self.marks.push(mark_renderer);
    }

    fn clear_mark_renderer(&mut self) {
        self.marks.clear();
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
        self.config.format
    }

    fn sample_count(&self) -> u32 {
        self.sample_count
    }
}
