use image::imageops::crop_imm;
use std::sync::Arc;
use wgpu::{
    Adapter, Buffer, BufferAddress, BufferDescriptor, BufferUsages, CommandBuffer,
    CommandEncoderDescriptor, Device, DeviceDescriptor, Extent3d, ImageCopyBuffer,
    ImageCopyTexture, ImageDataLayout, LoadOp, MapMode, Operations, Origin3d, PowerPreference,
    Queue, RenderPassColorAttachment, RenderPassDescriptor, RequestAdapterOptions, StoreOp,
    Surface, SurfaceConfiguration, Texture, TextureAspect, TextureDescriptor, TextureDimension,
    TextureFormat, TextureFormatFeatureFlags, TextureUsages, TextureView, TextureViewDescriptor,
};
use winit::dpi::Size;
use winit::event::WindowEvent;
use winit::window::Window;

use crate::error::AvengerWgpuError;
use crate::marks::instanced_mark::InstancedMarkRenderer;
use crate::marks::multi::MultiMarkRenderer;
use crate::marks::symbol::SymbolShader;
use avenger::marks::arc::ArcMark;
use avenger::marks::area::AreaMark;
use avenger::marks::group::Clip;
use avenger::marks::image::ImageMark;
use avenger::marks::line::LineMark;
use avenger::marks::path::PathMark;
use avenger::marks::trail::TrailMark;
use avenger::{
    marks::group::SceneGroup, marks::mark::SceneMark, marks::rect::RectMark, marks::rule::RuleMark,
    marks::symbol::SymbolMark, marks::text::TextMark, scene_graph::SceneGraph,
};

pub enum MarkRenderer {
    Instanced(InstancedMarkRenderer),
    Multi(Box<MultiMarkRenderer>),
}

#[derive(Debug, Copy, Clone)]
pub struct CanvasDimensions {
    pub size: [f32; 2],
    pub scale: f32,
}

impl CanvasDimensions {
    pub fn to_physical_width(&self) -> u32 {
        (self.size[0] * self.scale) as u32
    }

    pub fn to_physical_height(&self) -> u32 {
        (self.size[1] * self.scale) as u32
    }

    pub fn to_physical_size(&self) -> winit::dpi::PhysicalSize<u32> {
        winit::dpi::PhysicalSize {
            width: self.to_physical_width(),
            height: self.to_physical_height(),
        }
    }
}

pub trait Canvas {
    fn add_mark_renderer(&mut self, mark_renderer: MarkRenderer);
    fn clear_mark_renderer(&mut self);
    fn device(&self) -> &Device;
    fn queue(&self) -> &Queue;
    fn dimensions(&self) -> CanvasDimensions;

    fn texture_format(&self) -> TextureFormat;

    fn sample_count(&self) -> u32;

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer;

    fn add_arc_mark(
        &mut self,
        mark: &ArcMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_arc_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_path_mark(
        &mut self,
        mark: &PathMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_path_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_line_mark(
        &mut self,
        mark: &LineMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_line_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_trail_mark(
        &mut self,
        mark: &TrailMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_trail_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_area_mark(
        &mut self,
        mark: &AreaMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_area_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_symbol_mark(
        &mut self,
        mark: &SymbolMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        if mark.len >= 10000
            && mark.gradients.is_empty()
            && matches!(group_clip, Clip::None | Clip::Rect { .. })
        {
            self.add_mark_renderer(MarkRenderer::Instanced(InstancedMarkRenderer::new(
                self.device(),
                self.texture_format(),
                self.sample_count(),
                Box::new(SymbolShader::from_symbol_mark(
                    mark,
                    self.dimensions(),
                    origin,
                )?),
                group_clip.maybe_clip(mark.clip),
                self.dimensions().scale,
            )));
        } else {
            self.get_multi_renderer()
                .add_symbol_mark(mark, origin, group_clip)?;
        }

        Ok(())
    }

    fn add_rect_mark(
        &mut self,
        mark: &RectMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_rect_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_rule_mark(
        &mut self,
        mark: &RuleMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_rule_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_text_mark(
        &mut self,
        mark: &TextMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        cfg_if::cfg_if! {
            if #[cfg(feature = "cosmic-text")] {
                self.get_multi_renderer().add_text_mark(mark, origin, group_clip)?;
                Ok(())
            } else {
                Err(AvengerWgpuError::TextNotEnabled("Use the cosmic-text feature flag to enable text".to_string()))
            }
        }
    }

    fn add_image_mark(
        &mut self,
        mark: &ImageMark,
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
        // Maybe add rect around group boundary
        if let Some(rect) = group.make_path_mark() {
            self.add_path_mark(&rect, parent_origin, &group.clip)?;
        }

        // Add groups in order of zindex
        let zindex = group.marks.iter().map(|m| m.zindex()).collect::<Vec<_>>();
        let mut indices: Vec<usize> = (0..zindex.len()).collect();
        indices.sort_by_key(|i| zindex[*i].unwrap_or(0));

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

        for mark_ind in indices {
            let mark = &group.marks[mark_ind];
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
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn set_scene(&mut self, scene_graph: &SceneGraph) -> Result<(), AvengerWgpuError> {
        // Clear existing marks
        self.clear_mark_renderer();

        // Sort groups by zindex
        let zindex = scene_graph
            .groups
            .iter()
            .map(|g| g.zindex)
            .collect::<Vec<_>>();
        let mut indices: Vec<usize> = (0..zindex.len()).collect();
        indices.sort_by_key(|i| zindex[*i].unwrap_or(0));

        for group_ind in &indices {
            let group = &scene_graph.groups[*group_ind];
            self.add_group_mark(group, scene_graph.origin, &Clip::None)?;
        }

        Ok(())
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
    compatible_surface: Option<&Surface<'_>>,
) -> Result<Adapter, AvengerWgpuError> {
    instance
        .request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::default(),
            compatible_surface,
            force_fallback_adapter: false,
        })
        .await
        .ok_or(AvengerWgpuError::MakeWgpuAdapterError)
}

async fn request_wgpu_device(adapter: &Adapter) -> Result<(Device, Queue), AvengerWgpuError> {
    Ok(adapter
        .request_device(
            &DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                // WebGL doesn't support all of wgpu's features, so if
                // we're building for the web we'll have to disable some.
                required_limits: if cfg!(target_arch = "wasm32") {
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

pub struct WindowCanvas<'window> {
    window: Arc<Window>,
    surface: Surface<'window>,
    device: Device,
    queue: Queue,
    multisampled_framebuffer: TextureView,
    sample_count: u32,
    config: SurfaceConfiguration,
    dimensions: CanvasDimensions,
    marks: Vec<MarkRenderer>,
    multi_renderer: Option<MultiMarkRenderer>,
}

impl<'window> WindowCanvas<'window> {
    pub async fn new(
        window: Window,
        dimensions: CanvasDimensions,
    ) -> Result<Self, AvengerWgpuError> {
        let _ = window.request_inner_size(Size::Physical(dimensions.to_physical_size()));
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

        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: dimensions.to_physical_width(),
            height: dimensions.to_physical_height(),
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
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
            window,
            marks: Vec::new(),
            multi_renderer: None,
        })
    }

    pub fn get_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.dimensions.to_physical_size()
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, _new_size: winit::dpi::PhysicalSize<u32>) {
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

impl<'window> Canvas for WindowCanvas<'window> {
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

pub struct PngCanvas {
    device: Device,
    queue: Queue,
    multisampled_framebuffer: TextureView,
    sample_count: u32,
    marks: Vec<MarkRenderer>,
    pub dimensions: CanvasDimensions,
    pub texture_view: TextureView,
    pub output_buffer: Buffer,
    pub texture: Texture,
    pub texture_size: Extent3d,
    pub padded_width: u32,
    pub padded_height: u32,
    multi_renderer: Option<MultiMarkRenderer>,
}

impl PngCanvas {
    #[tracing::instrument(skip_all)]
    pub async fn new(dimensions: CanvasDimensions) -> Result<Self, AvengerWgpuError> {
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
            multi_renderer: None,
        })
    }

    #[tracing::instrument(skip_all)]
    pub async fn render(&mut self) -> Result<image::RgbaImage, AvengerWgpuError> {
        // Commit open multi mark renderer
        if let Some(multi_renderer) = self.multi_renderer.take() {
            self.marks
                .push(MarkRenderer::Multi(Box::new(multi_renderer)));
        }

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
        let texture_format = self.texture_format();
        for mark in &mut self.marks {
            let command = match mark {
                MarkRenderer::Instanced(renderer) => {
                    if self.sample_count > 1 {
                        renderer.render(
                            &self.device,
                            &self.multisampled_framebuffer,
                            Some(&self.texture_view),
                        )
                    } else {
                        renderer.render(&self.device, &self.texture_view, None)
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
                            Some(&self.texture_view),
                        )
                    } else {
                        renderer.render(
                            &self.device,
                            &self.queue,
                            texture_format,
                            self.sample_count,
                            &self.texture_view,
                            None,
                        )
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

            let cropped_img = crop_imm(
                &img_buf,
                0,
                0,
                self.dimensions.to_physical_width(),
                self.dimensions.to_physical_height(),
            );
            cropped_img.to_image()
        };

        self.output_buffer.unmap();
        Ok(img)
    }
}

impl Canvas for PngCanvas {
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
        self.texture.format()
    }

    fn sample_count(&self) -> u32 {
        self.sample_count
    }
}
