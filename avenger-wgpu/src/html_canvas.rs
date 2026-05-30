use std::{collections::HashMap, sync::Arc};

use avenger_common::canvas::CanvasDimensions;
use web_sys::HtmlCanvasElement;
use wgpu::{
    Device, Extent3d, Queue, Surface, SurfaceConfiguration, SurfaceTarget, TextureFormat,
    TextureUsages, TextureView, TextureViewDescriptor,
};

use crate::{
    canvas::{
        create_multisampled_framebuffer, get_supported_sample_count, make_background_command,
        make_text_atlas_builder, make_wgpu_adapter, request_wgpu_device, Canvas, CanvasConfig,
        CanvasDimensionUtils, MarkRenderer,
    },
    error::AvengerWgpuError,
    marks::{
        instanced_mark::InstancedMarkRenderer,
        multi::{MultiMarkRenderResources, MultiMarkRenderer},
        text::TextAtlasBuilderTrait,
    },
};

pub struct HtmlCanvasCanvas<'window> {
    sample_count: u32,
    surface_config: SurfaceConfiguration,
    dimensions: CanvasDimensions,
    marks: Vec<MarkRenderer>,
    // One shared multi-renderer; each run of multi-marks is recorded in `marks` as a
    // batch range. `run_start` is the batch index where the current run began.
    shared_multi: MultiMarkRenderer,
    run_start: usize,
    instanced_renderers: HashMap<u64, Arc<InstancedMarkRenderer>>,
    multi_render_resources: MultiMarkRenderResources,
    config: CanvasConfig,
    // Text atlas shared by all multi-renderers; built + uploaded once per frame.
    text_atlas_builder: Box<dyn TextAtlasBuilderTrait>,

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
        let sample_count = get_supported_sample_count(format_flags);
        let multisampled_framebuffer = create_multisampled_framebuffer(
            &device,
            surface_config.width,
            surface_config.height,
            surface_format,
            sample_count,
        );

        let multi_render_resources =
            MultiMarkRenderResources::new(&device, surface_format, sample_count);

        let text_atlas_builder = make_text_atlas_builder(&config.text_builder_ctor);

        Ok(Self {
            surface,
            device,
            queue,
            multisampled_framebuffer,
            sample_count,
            surface_config,
            dimensions,
            marks: Vec::new(),
            shared_multi: MultiMarkRenderer::new(dimensions),
            run_start: 0,
            instanced_renderers: HashMap::new(),
            multi_render_resources,
            config,
            text_atlas_builder,
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
        let output_extent = output.texture.size();
        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());

        // Commit the final run of multi-marks as a batch range into the shared renderer.
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(MarkRenderer::Multi {
                batch_range: self.run_start..end,
            });
        }
        self.run_start = end;

        let background_command = if self.sample_count > 1 {
            make_background_command(self, &self.multisampled_framebuffer, Some(&view))
        } else {
            make_background_command(self, &view, None)
        };
        let mut commands = vec![background_command];
        let render_target_extent = if self.sample_count > 1 {
            Extent3d {
                width: self.surface_config.width,
                height: self.surface_config.height,
                depth_or_array_layers: 1,
            }
        } else {
            output_extent
        };

        // Build the shared text atlas bind groups ONCE per frame (instead of once per
        // multi-renderer). Every renderer this frame registered into the same atlas, so
        // these bind groups are page-correct for all of them.
        let (text_atlas_size, text_atlas_images) = self.text_atlas_builder.build();
        let text_bind_groups = MultiMarkRenderer::make_text_bind_groups_dual_sampler(
            &self.device,
            &self.queue,
            self.multi_render_resources.text_layout(),
            text_atlas_size,
            &text_atlas_images,
        );

        // Prepare the single shared multi-renderer ONCE per frame; each run is encoded
        // from this shared prep.
        let prepared = self.shared_multi.prepare(
            &self.device,
            &self.queue,
            render_target_extent,
            &self.multi_render_resources,
        );

        // Coalesce consecutive multi-mark runs (document order) into one command
        // buffer with merged passes; instanced marks break a run.
        let mut pending: Vec<std::ops::Range<usize>> = Vec::new();
        for mark in &self.marks {
            match mark {
                MarkRenderer::Multi { batch_range } => pending.push(batch_range.clone()),
                MarkRenderer::Instanced {
                    renderer,
                    x_adjustment,
                    y_adjustment,
                } => {
                    if !pending.is_empty() {
                        let c = if self.sample_count > 1 {
                            self.shared_multi.encode_multi_ranges(
                                &self.device,
                                render_target_extent,
                                &self.multisampled_framebuffer,
                                Some(&view),
                                &self.multi_render_resources,
                                &text_bind_groups,
                                &prepared,
                                &pending,
                            )
                        } else {
                            self.shared_multi.encode_multi_ranges(
                                &self.device,
                                render_target_extent,
                                &view,
                                None,
                                &self.multi_render_resources,
                                &text_bind_groups,
                                &prepared,
                                &pending,
                            )
                        };
                        commands.push(c);
                        pending.clear();
                    }
                    let c = if self.sample_count > 1 {
                        renderer.render(
                            &self.device,
                            &self.multisampled_framebuffer,
                            Some(&view),
                            *x_adjustment,
                            *y_adjustment,
                        )
                    } else {
                        renderer.render(&self.device, &view, None, *x_adjustment, *y_adjustment)
                    };
                    commands.push(c);
                }
            }
        }
        if !pending.is_empty() {
            let c = if self.sample_count > 1 {
                self.shared_multi.encode_multi_ranges(
                    &self.device,
                    render_target_extent,
                    &self.multisampled_framebuffer,
                    Some(&view),
                    &self.multi_render_resources,
                    &text_bind_groups,
                    &prepared,
                    &pending,
                )
            } else {
                self.shared_multi.encode_multi_ranges(
                    &self.device,
                    render_target_extent,
                    &view,
                    None,
                    &self.multi_render_resources,
                    &text_bind_groups,
                    &prepared,
                    &pending,
                )
            };
            commands.push(c);
        }

        self.queue.submit(commands);
        output.present();

        Ok(())
    }
}

impl<'window> Canvas for HtmlCanvasCanvas<'window> {
    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        &mut self.shared_multi
    }

    fn text_atlas_builder(&mut self) -> &mut dyn TextAtlasBuilderTrait {
        &mut *self.text_atlas_builder
    }

    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<avenger_common::types::LinearScaleAdjustment>,
        y_adjustment: Option<avenger_common::types::LinearScaleAdjustment>,
    ) {
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(MarkRenderer::Multi {
                batch_range: self.run_start..end,
            });
        }
        self.run_start = end;
        self.instanced_renderers
            .insert(fingerprint, mark_renderer.clone());
        self.marks.push(MarkRenderer::Instanced {
            renderer: mark_renderer,
            x_adjustment,
            y_adjustment,
        });
    }

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.instanced_renderers.get(&fingerprint).cloned()
    }

    fn clear_mark_renderer(&mut self) {
        self.shared_multi.reset_for_frame(self.dimensions);
        self.run_start = 0;
        self.marks.clear();
        self.instanced_renderers.clear();

        // Reset the shared text atlas so each frame starts clean (matches the old
        // per-renderer reset-per-frame semantics).
        self.text_atlas_builder = make_text_atlas_builder(&self.config.text_builder_ctor);
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
