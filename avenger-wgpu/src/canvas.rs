use avenger_common::canvas::CanvasDimensions;
use avenger_common::types::LinearScaleAdjustment;
use image::imageops::crop_imm;
use std::collections::HashMap;
use std::sync::Arc;

use wgpu::{
    Adapter, Buffer, BufferAddress, BufferDescriptor, BufferUsages, CommandBuffer,
    CommandEncoderDescriptor, Device, DeviceDescriptor, Extent3d, LoadOp, MapMode, Operations,
    Origin3d, PowerPreference, Queue, RenderPassColorAttachment, RenderPassDescriptor,
    RequestAdapterOptions, StoreOp, Surface, SurfaceConfiguration, TexelCopyBufferInfo,
    TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureFormatFeatureFlags, TextureUsages, TextureView,
    TextureViewDescriptor, Trace,
};
use winit::dpi::Size;
use winit::event::WindowEvent;
use winit::window::Window;

use crate::error::AvengerWgpuError;
use crate::marks::instanced_mark::{InstancedMarkFingerprint, InstancedMarkRenderer};
use crate::marks::multi::MultiMarkRenderer;
use crate::marks::symbol::SymbolShader;
use crate::marks::text::TextAtlasBuilderTrait;
use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::area::SceneAreaMark;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::image::SceneImageMark;
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::trail::SceneTrailMark;
use avenger_scenegraph::{
    marks::group::SceneGroup, marks::mark::SceneMark, marks::rect::SceneRectMark,
    marks::rule::SceneRuleMark, marks::symbol::SceneSymbolMark, marks::text::SceneTextMark,
    scene_graph::SceneGraph,
};

pub enum MarkRenderer {
    Instanced {
        renderer: Arc<InstancedMarkRenderer>,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    },
    Multi(Box<MultiMarkRenderer>),
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

#[derive(Default)]
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

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>>;

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
        if mark.len >= 100
            && mark.gradients.is_empty()
            && matches!(group_clip, Clip::None | Clip::Rect { .. })
        {
            // Check if compatible renderer already exists
            let fingerprint = mark.instanced_fingerprint();
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
        self.get_multi_renderer()
            .add_text_mark(mark, origin, group_clip)?;
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
        let groups = scene_graph.groups();
        let zindex = groups.iter().map(|g| g.zindex).collect::<Vec<_>>();
        let mut indices: Vec<usize> = (0..zindex.len()).collect();
        indices.sort_by_key(|i| zindex[*i].unwrap_or(0));

        for group_ind in &indices {
            let group = groups[*group_ind];
            self.add_group_mark(group, scene_graph.origin, &Clip::None)?;
        }

        Ok(())
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
    instance
        .request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::default(),
            compatible_surface,
            force_fallback_adapter: false,
        })
        .await
        .map_err(|_| AvengerWgpuError::MakeWgpuAdapterError)
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
    marks: Vec<MarkRenderer>,
    multi_renderer: Option<MultiMarkRenderer>,
    instanced_renderers: HashMap<u64, Arc<InstancedMarkRenderer>>,
    config: CanvasConfig,

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
        let sample_count = get_supported_sample_count(format_flags);
        let multisampled_framebuffer = create_multisampled_framebuffer(
            &device,
            surface_config.width,
            surface_config.height,
            surface_format,
            sample_count,
        );

        // // Uncomment to capture GPU boundary
        // unsafe { device.start_graphics_debugger_capture() };

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
            multi_renderer: None,
            instanced_renderers: HashMap::new(),
            config,
        })
    }

    pub fn get_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.dimensions.to_physical_size()
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            // Update dimensions
            self.dimensions = CanvasDimensions {
                size: [
                    new_size.width as f32 / self.dimensions.scale,
                    new_size.height as f32 / self.dimensions.scale,
                ],
                scale: self.dimensions.scale,
            };

            // Update surface configuration
            self.surface_config.width = new_size.width;
            self.surface_config.height = new_size.height;
            self.surface.configure(&self.device, &self.surface_config);

            // Create new multisampled framebuffer with updated size
            self.multisampled_framebuffer = create_multisampled_framebuffer(
                &self.device,
                new_size.width,
                new_size.height,
                self.surface_config.format,
                self.sample_count,
            );
        }
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
                MarkRenderer::Instanced {
                    renderer,
                    x_adjustment,
                    y_adjustment,
                } => {
                    if self.sample_count > 1 {
                        renderer.render(
                            &self.device,
                            &self.multisampled_framebuffer,
                            Some(&view),
                            *x_adjustment,
                            *y_adjustment,
                        )
                    } else {
                        renderer.render(&self.device, &view, None, *x_adjustment, *y_adjustment)
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

impl Canvas for WindowCanvas<'_> {
    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        if self.multi_renderer.is_none() {
            self.multi_renderer = Some(MultiMarkRenderer::new(
                self.dimensions,
                self.config.text_builder_ctor.clone(),
            ));
        }
        self.multi_renderer.as_mut().unwrap()
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
        if let Some(multi_renderer) = self.multi_renderer.take() {
            self.marks
                .push(MarkRenderer::Multi(Box::new(multi_renderer)));
        }
        self.instanced_renderers
            .insert(fingerprint, mark_renderer.clone());
        self.marks.push(MarkRenderer::Instanced {
            renderer: mark_renderer,
            x_adjustment,
            y_adjustment,
        });
    }

    fn clear_mark_renderer(&mut self) {
        self.get_multi_renderer().clear();
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
    marks: Vec<MarkRenderer>,
    dimensions: CanvasDimensions,
    texture_view: TextureView,
    output_buffer: Buffer,
    texture: Texture,
    texture_size: Extent3d,
    padded_width: u32,
    padded_height: u32,
    multi_renderer: Option<MultiMarkRenderer>,
    instanced_renderers: HashMap<u64, Arc<InstancedMarkRenderer>>,
    config: CanvasConfig,

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
            instanced_renderers: HashMap::new(),
            config,
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
                MarkRenderer::Instanced {
                    renderer,
                    x_adjustment,
                    y_adjustment,
                } => {
                    if self.sample_count > 1 {
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

        // Output to png file
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

        self.output_buffer.unmap();
        Ok(img)
    }
}

impl Canvas for PngCanvas {
    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        if self.multi_renderer.is_none() {
            self.multi_renderer = Some(MultiMarkRenderer::new(
                self.dimensions,
                self.config.text_builder_ctor.clone(),
            ));
        }
        self.multi_renderer.as_mut().unwrap()
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
        if let Some(multi_renderer) = self.multi_renderer.take() {
            self.marks
                .push(MarkRenderer::Multi(Box::new(multi_renderer)));
        }
        self.instanced_renderers
            .insert(fingerprint, mark_renderer.clone());
        self.marks.push(MarkRenderer::Instanced {
            renderer: mark_renderer,
            x_adjustment,
            y_adjustment,
        });
    }

    fn clear_mark_renderer(&mut self) {
        self.get_multi_renderer().clear();
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
