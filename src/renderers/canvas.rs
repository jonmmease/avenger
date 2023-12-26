use image::imageops::crop_imm;
use winit::event::WindowEvent;
use winit::window::Window;
use crate::renderers::MarkRenderer;
use crate::renderers::rect::RectMarkRenderer;
use crate::renderers::symbol::SymbolMarkRenderer;
use crate::scene::rect::{RectInstance, RectMark};
use crate::scene::scene_graph::{SceneGraph, SceneGroup, SceneMark};
use crate::scene::symbol::{SymbolInstance, SymbolMark};

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CanvasUniform {
    size: [f32; 2],
    origin: [f32; 2],
}

pub struct Canvas {
    window: Window,
    surface: wgpu::Surface,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    marks: Vec<MarkRenderer>,
    uniform: CanvasUniform,
    origin: [f32; 2],
}

impl Canvas {
    pub async fn new(window: Window, origin: [f32; 2]) -> Self {
        let size = window.inner_size();

        // The instance is a handle to our GPU
        // BackendBit::PRIMARY => Vulkan + Metal + DX12 + Browser WebGPU
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // # Safety
        //
        // The surface needs to live as long as the window that created it.
        // State owns the window so this should be safe.
        let surface = unsafe { instance.create_surface(&window) }.unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
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
                // Some(&std::path::Path::new("trace")), // Trace path
                None,
            )
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);

        // Select first non-srgb texture format
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        let uniform = CanvasUniform {
            size: [size.width as f32, size.height as f32],
            origin: origin.clone(),
        };

        Self {
            surface,
            device,
            queue,
            config,
            size,
            window,
            uniform,
            marks: Vec::new(),
            origin,
        }
    }

    pub fn get_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.size
    }

    pub fn add_symbol_mark(&mut self, mark: &SymbolMark) {
        self.marks.push(MarkRenderer::Symbol(SymbolMarkRenderer::new(
            &self.device, self.uniform.clone(), self.config.format, mark.instances.as_slice()
        )));
    }

    pub fn add_rect_mark(&mut self, mark: &RectMark) {
        self.marks.push(MarkRenderer::Rect(RectMarkRenderer::new(
            &self.device, self.uniform.clone(), self.config.format, mark.instances.as_slice()
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
                SceneMark::Group(group) => {
                    self.add_group_mark(group);
                }
            }
        }
    }

    pub fn set_scene(&mut self, scene_graph: &SceneGraph) {
        // Set uniforms
        self.uniform = CanvasUniform {
            size: [scene_graph.width, scene_graph.height],
            origin: scene_graph.origin,
        };

        // Clear existing marks
        self.marks.clear();

        // Add marks
        for group in &scene_graph.groups {
            self.add_group_mark(group);
        }
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

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());


        // Build encoder for chart background
        let mut background_encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Background Encoder"),
            });

        {
            let _render_pass = background_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }
        let mut commands = vec![background_encoder.finish()];

        for mark in &self.marks {
            let command = match mark {
                MarkRenderer::Symbol(mark) => {
                    mark.render(&self.device, &view)
                }
                MarkRenderer::Rect(mark) => {
                    mark.render(&self.device, &view)
                }
            };
            commands.push(command);
        }

        self.queue.submit(commands);
        output.present();

        Ok(())
    }

}


pub struct PngCanvas {
    device: wgpu::Device,
    queue: wgpu::Queue,
    marks: Vec<MarkRenderer>,
    uniform: CanvasUniform,
    width: f32,
    height: f32,
    origin: [f32; 2],
    pub texture_view: wgpu::TextureView,
    pub output_buffer: wgpu::Buffer,
    pub texture: wgpu::Texture,
    pub texture_size: wgpu::Extent3d,
    pub padded_width: u32,
    pub padded_height: u32,
}


impl PngCanvas {
    pub async fn new(width: f32, height: f32, origin: [f32; 2]) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
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
                // Some(&std::path::Path::new("trace")), // Trace path
                None,
            )
            .await
            .unwrap();

        let texture_desc = wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT
            ,
            label: None,
            view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
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

        let output_buffer_size = (u32_size * padded_width * padded_height) as wgpu::BufferAddress;
        let output_buffer_desc = wgpu::BufferDescriptor {
            size: output_buffer_size,
            usage: wgpu::BufferUsages::COPY_DST
                // this tells wpgu that we want to read this buffer from the cpu
                | wgpu::BufferUsages::MAP_READ,
            label: None,
            mapped_at_creation: false,
        };
        let output_buffer = device.create_buffer(&output_buffer_desc);

        let uniform = CanvasUniform {
            size: [width, height],
            origin: origin.clone(),
        };

        Self {
            device,
            queue,
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
        }
    }

    pub async fn render(&mut self) -> Result<image::RgbaImage, wgpu::SurfaceError> {
        // let output = self.surface.get_current_texture()?;
        // let view = output
        //     .texture
        //     .create_view(&wgpu::TextureViewDescriptor::default());
        //
        //
        // Build encoder for chart background
        let mut background_encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Background Encoder"),
            });

        {
            let _render_pass = background_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }
        let mut commands = vec![background_encoder.finish()];

        for mark in &self.marks {
            let command = match mark {
                MarkRenderer::Symbol(mark) => {
                    mark.render(&self.device, &self.texture_view)
                }
                MarkRenderer::Rect(mark) => {
                    mark.render(&self.device, &self.texture_view)
                }
            };
            commands.push(command);
        }

        self.queue.submit(commands);

        // Extract texture from GPU
        let mut extract_encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Extract Texture Encoder"),
            });

        let u32_size = std::mem::size_of::<u32>() as u32;

        extract_encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                aspect: wgpu::TextureAspect::All,
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self.output_buffer,
                layout: wgpu::ImageDataLayout {
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
            buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
            self.device.poll(wgpu::Maintain::Wait);

            // TODO: remove panic
            rx.receive().await.unwrap().unwrap();

            let data = buffer_slice.get_mapped_range();
            let img_buf = image::RgbaImage::from_vec(self.padded_width, self.padded_height, data.to_vec()).unwrap();

            // use image::{ImageBuffer, Rgba};
            // let buffer =
            //     ImageBuffer::<Rgba<u8>, _>::from_raw(self.padded_width, self.padded_height, data).unwrap();
            //
            let cropped_img = crop_imm(&img_buf, 0, 0, self.width as u32, self.height as u32);
            // cropped_img.
            // cropped_img.to_image().save("image.png").unwrap();
            // buffer.save("image.png").unwrap();
            cropped_img.to_image()
        };

        self.output_buffer.unmap();
        Ok((img))
    }

    pub fn add_symbol_mark(&mut self, mark: &SymbolMark) {
        self.marks.push(MarkRenderer::Symbol(SymbolMarkRenderer::new(
            &self.device, self.uniform.clone(), self.texture.format(), mark.instances.as_slice()
        )));
    }

    pub fn add_rect_mark(&mut self, mark: &RectMark) {
        self.marks.push(MarkRenderer::Rect(RectMarkRenderer::new(
            &self.device, self.uniform.clone(), self.texture.format(), mark.instances.as_slice()
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
                SceneMark::Group(group) => {
                    self.add_group_mark(group);
                }
            }
        }
    }

    pub fn set_scene(&mut self, scene_graph: &SceneGraph) {
        // Set uniforms
        self.uniform = CanvasUniform {
            size: [scene_graph.width, scene_graph.height],
            origin: scene_graph.origin,
        };

        // Clear existing marks
        self.marks.clear();

        // Add marks
        for group in &scene_graph.groups {
            self.add_group_mark(group);
        }
    }
}