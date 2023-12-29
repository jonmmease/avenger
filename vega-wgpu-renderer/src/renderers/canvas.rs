use crate::error::VegaWgpuError;
use crate::renderers::mark::GeomMarkRenderer;
use crate::renderers::rect::RectShader;
use crate::renderers::rule::RuleShader;
use crate::renderers::symbol::SymbolShader;
use crate::renderers::text::TextMarkRenderer;
use crate::scene::rect::{RectInstance, RectMark};
use crate::scene::rule::RuleMark;
use crate::scene::scene_graph::{SceneGraph, SceneGroup, SceneMark};
use crate::scene::symbol::{SymbolInstance, SymbolMark};
use crate::scene::text::TextMark;
use image::imageops::crop_imm;
use wgpu::{
    Adapter, Buffer, BufferAddress, BufferDescriptor, BufferUsages, CommandBuffer, CommandEncoder,
    CommandEncoderDescriptor, Device, DeviceDescriptor, Extent3d, ImageCopyBuffer,
    ImageCopyTexture, ImageDataLayout, LoadOp, MapMode, Operations, Origin3d, PowerPreference,
    Queue, RenderPassColorAttachment, RenderPassDescriptor, RequestAdapterOptions, StoreOp,
    Surface, SurfaceConfiguration, SurfaceError, Texture, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureFormatFeatureFlags, TextureUsages, TextureView,
    TextureViewDescriptor,
};
use winit::event::WindowEvent;
use winit::window::Window;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CanvasUniform {
    pub size: [f32; 2],
    filler: [f32; 2], // Pad to 16 bytes
}

pub enum MarkRenderer {
    Geom(GeomMarkRenderer),
    Text(TextMarkRenderer),
}

pub trait Canvas {
    fn add_mark_renderer(&mut self, mark_renderer: MarkRenderer);
    fn clear_mark_renderer(&mut self);
    fn device(&self) -> &Device;
    fn queue(&self) -> &Queue;
    fn uniform(&self) -> &CanvasUniform;

    fn set_uniform(&mut self, uniform: CanvasUniform);

    fn texture_format(&self) -> TextureFormat;

    fn sample_count(&self) -> u32;

    fn add_symbol_mark(&mut self, mark: &SymbolMark) {
        self.add_mark_renderer(MarkRenderer::Geom(GeomMarkRenderer::new(
            &self.device(),
            self.uniform().clone(),
            self.texture_format(),
            self.sample_count(),
            Box::new(SymbolShader::new(mark.shape)),
            mark.instances.as_slice(),
        )));
    }

    fn add_rect_mark(&mut self, mark: &RectMark) {
        self.add_mark_renderer(MarkRenderer::Geom(GeomMarkRenderer::new(
            &self.device(),
            self.uniform().clone(),
            self.texture_format(),
            self.sample_count(),
            Box::new(RectShader::new()),
            mark.instances.as_slice(),
        )));
    }

    fn add_rule_mark(&mut self, mark: &RuleMark) {
        self.add_mark_renderer(MarkRenderer::Geom(GeomMarkRenderer::new(
            &self.device(),
            self.uniform().clone(),
            self.texture_format(),
            self.sample_count(),
            Box::new(RuleShader::new()),
            mark.instances.as_slice(),
        )));
    }

    fn add_text_mark(&mut self, mark: &TextMark) {
        self.add_mark_renderer(MarkRenderer::Text(TextMarkRenderer::new(
            &self.device(),
            &self.queue(),
            self.uniform().clone(),
            self.texture_format(),
            self.sample_count(),
            mark.instances.clone(),
        )));
    }

    fn add_group_mark(&mut self, group: &SceneGroup) {
        for mark in &group.marks {
            match mark {
                SceneMark::Symbol(mark) => {
                    self.add_symbol_mark(mark);
                }
                SceneMark::Rect(mark) => {
                    self.add_rect_mark(mark);
                }
                SceneMark::Rule(mark) => {
                    self.add_rule_mark(mark);
                }
                SceneMark::Text(mark) => {
                    self.add_text_mark(mark);
                }
                SceneMark::Group(group) => {
                    self.add_group_mark(group);
                }
            }
        }
    }

    fn set_scene(&mut self, scene_graph: &SceneGraph) {
        // Set uniforms
        self.set_uniform(CanvasUniform {
            size: [scene_graph.width, scene_graph.height],
            filler: [0.0, 0.0],
        });

        // Clear existing marks
        self.clear_mark_renderer();

        // Add marks
        for group in &scene_graph.groups {
            self.add_group_mark(group);
        }
    }
}

// Private shared canvas logic
fn make_background_command<C: Canvas>(
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

fn make_wgpu_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    })
}

async fn make_wgpu_adapter(
    instance: &wgpu::Instance,
    compatible_surface: Option<&Surface>,
) -> Result<Adapter, VegaWgpuError> {
    instance
        .request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::default(),
            compatible_surface,
            force_fallback_adapter: false,
        })
        .await
        .ok_or(VegaWgpuError::MakeWgpuAdapterError)
}

async fn request_wgpu_device(adapter: &Adapter) -> Result<(Device, Queue), VegaWgpuError> {
    Ok(adapter
        .request_device(
            &DeviceDescriptor {
                label: None,
                features: wgpu::Features::empty(),
                // WebGL doesn't support all of wgpu's features, so if
                // we're building for the web we'll have to disable some.
                limits: if cfg!(target_arch = "wasm32") {
                    wgpu::Limits::downlevel_webgl2_defaults()
                } else {
                    wgpu::Limits::default()
                },
            },
            None,
        )
        .await?)
}

fn create_multisampled_framebuffer(
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

fn get_supported_sample_count(sample_flags: TextureFormatFeatureFlags) -> u32 {
    // Get max supported sample count up to 4
    if sample_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4) {
        4
    } else if sample_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X2) {
        2
    } else {
        1
    }
}

pub struct WindowCanvas {
    window: Window,
    surface: Surface,
    device: Device,
    queue: Queue,
    multisampled_framebuffer: TextureView,
    sample_count: u32,
    config: SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    marks: Vec<MarkRenderer>,
    uniform: CanvasUniform,
    origin: [f32; 2],
}

impl WindowCanvas {
    pub async fn new(window: Window, origin: [f32; 2]) -> Result<Self, VegaWgpuError> {
        let size = window.inner_size();

        let instance = make_wgpu_instance();
        let surface = unsafe { instance.create_surface(&window) }?;
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
            width: size.width,
            height: size.height,
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

        let uniform = CanvasUniform {
            size: [size.width as f32, size.height as f32],
            filler: [0.0, 0.0],
        };

        Ok(Self {
            surface,
            device,
            queue,
            multisampled_framebuffer,
            sample_count,
            config,
            size,
            window,
            uniform,
            marks: Vec::new(),
            origin,
        })
    }

    pub fn get_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.size
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        // if new_size.width > 0 && new_size.height > 0 {
        //     self.size = new_size;
        //     self.config.width = new_size.width;
        //     self.config.height = new_size.height;
        //     self.surface.configure(&self.device, &self.config);
        // }
    }

    #[allow(unused_variables)]
    pub fn input(&mut self, event: &WindowEvent) -> bool {
        false
    }

    pub fn update(&mut self) {}

    pub fn render(&mut self) -> Result<(), SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());

        let background_command = if self.sample_count > 1 {
            make_background_command(self, &self.multisampled_framebuffer, Some(&view))
        } else {
            make_background_command(self, &view, None)
        };
        let mut commands = vec![background_command];

        for mark in &mut self.marks {
            let command = match mark {
                MarkRenderer::Geom(mark) => {
                    if self.sample_count > 1 {
                        mark.render(&self.device, &self.multisampled_framebuffer, Some(&view))
                    } else {
                        mark.render(&self.device, &view, None)
                    }
                }
                MarkRenderer::Text(mark) => {
                    if self.sample_count > 1 {
                        mark.render(
                            &self.device,
                            &self.queue,
                            &self.multisampled_framebuffer,
                            Some(&view),
                        )
                    } else {
                        mark.render(&self.device, &self.queue, &view, None)
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

impl Canvas for WindowCanvas {
    fn add_mark_renderer(&mut self, mark_renderer: MarkRenderer) {
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

    fn uniform(&self) -> &CanvasUniform {
        &self.uniform
    }

    fn set_uniform(&mut self, uniform: CanvasUniform) {
        self.uniform = uniform;
    }

    fn texture_format(&self) -> TextureFormat {
        self.config.format
    }

    fn sample_count(&self) -> u32 {
        self.sample_count
    }
}

pub struct PngCanvas {
    device: Device,
    queue: Queue,
    multisampled_framebuffer: TextureView,
    sample_count: u32,
    marks: Vec<MarkRenderer>,
    uniform: CanvasUniform,
    width: f32,
    height: f32,
    origin: [f32; 2],
    pub texture_view: TextureView,
    pub output_buffer: Buffer,
    pub texture: Texture,
    pub texture_size: Extent3d,
    pub padded_width: u32,
    pub padded_height: u32,
}

impl PngCanvas {
    pub async fn new(width: f32, height: f32, origin: [f32; 2]) -> Result<Self, VegaWgpuError> {
        let instance = make_wgpu_instance();
        let adapter = make_wgpu_adapter(&instance, None).await?;
        let (device, queue) = request_wgpu_device(&adapter).await?;
        let texture_format = TextureFormat::Rgba8Unorm;
        let format_flags = adapter.get_texture_format_features(texture_format).flags;
        let sample_count = get_supported_sample_count(format_flags);

        let texture_desc = TextureDescriptor {
            size: Extent3d {
                width: width as u32,
                height: height as u32,
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
        let padded_width = (256.0 * (width / 256.0).ceil()) as u32;
        let padded_height = (256.0 * (width / 256.0).ceil()) as u32;

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

        let uniform = CanvasUniform {
            size: [width, height],
            filler: [0.0, 0.0],
        };

        let multisampled_framebuffer = create_multisampled_framebuffer(
            &device,
            width as u32,
            height as u32,
            texture_format,
            sample_count,
        );

        Ok(Self {
            device,
            queue,
            multisampled_framebuffer,
            sample_count,
            width,
            height,
            uniform,
            origin,
            texture,
            texture_view,
            output_buffer,
            texture_size,
            padded_width,
            padded_height,
            marks: Vec::new(),
        })
    }

    pub async fn render(&mut self) -> Result<image::RgbaImage, SurfaceError> {
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

        let mut commands = vec![background_command];

        for mark in &mut self.marks {
            let command = match mark {
                MarkRenderer::Geom(mark) => {
                    if self.sample_count > 1 {
                        mark.render(
                            &self.device,
                            &self.multisampled_framebuffer,
                            Some(&self.texture_view),
                        )
                    } else {
                        mark.render(&self.device, &self.texture_view, None)
                    }
                }
                MarkRenderer::Text(mark) => {
                    if self.sample_count > 1 {
                        mark.render(
                            &self.device,
                            &self.queue,
                            &self.multisampled_framebuffer,
                            Some(&self.texture_view),
                        )
                    } else {
                        mark.render(&self.device, &self.queue, &self.texture_view, None)
                    }
                }
            };

            commands.push(command);
        }

        self.queue.submit(commands);

        // Extract texture from GPU
        let mut extract_encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Extract Texture Encoder"),
            });

        let u32_size = std::mem::size_of::<u32>() as u32;

        extract_encoder.copy_texture_to_buffer(
            ImageCopyTexture {
                aspect: TextureAspect::All,
                texture: &self.texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
            },
            ImageCopyBuffer {
                buffer: &self.output_buffer,
                layout: ImageDataLayout {
                    offset: 0,
                    // bytes_per_row: Some(u32_size * self.width as u32),
                    bytes_per_row: Some(u32_size * self.padded_width),
                    rows_per_image: Some(self.padded_height),
                },
            },
            self.texture_size,
        );
        self.queue.submit(Some(extract_encoder.finish()));

        // Output to png file
        let img = {
            let buffer_slice = self.output_buffer.slice(..);

            // NOTE: We have to create the mapping THEN device.poll() before await
            // the future. Otherwise the application will freeze.
            let (tx, rx) = futures_intrusive::channel::shared::oneshot_channel();
            buffer_slice.map_async(MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
            self.device.poll(wgpu::Maintain::Wait);

            // TODO: remove panic
            rx.receive().await.unwrap().unwrap();

            let data = buffer_slice.get_mapped_range();
            let img_buf =
                image::RgbaImage::from_vec(self.padded_width, self.padded_height, data.to_vec())
                    .unwrap();

            let cropped_img = crop_imm(&img_buf, 0, 0, self.width as u32, self.height as u32);
            cropped_img.to_image()
        };

        self.output_buffer.unmap();
        Ok((img))
    }
}

impl Canvas for PngCanvas {
    fn add_mark_renderer(&mut self, mark_renderer: MarkRenderer) {
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

    fn uniform(&self) -> &CanvasUniform {
        &self.uniform
    }

    fn set_uniform(&mut self, uniform: CanvasUniform) {
        self.uniform = uniform;
    }

    fn texture_format(&self) -> TextureFormat {
        self.texture.format()
    }

    fn sample_count(&self) -> u32 {
        self.sample_count
    }
}
