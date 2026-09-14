use std::ops::Range;

use avenger_common::{time::Instant, types::LinearScaleAdjustment};
use avenger_scenegraph::marks::group::Clip;
use wgpu::{
    util::DeviceExt, CommandBuffer, Device, Extent3d, TexelCopyBufferLayout, TextureFormat,
    TextureView,
};

#[derive(Clone)]
pub struct InstancedMarkBatch {
    pub instances_range: Range<u32>,
    pub image: Option<image::DynamicImage>,
}

pub trait InstancedMarkShader {
    type Instance: bytemuck::Pod + bytemuck::Zeroable;
    type Vertex: bytemuck::Pod + bytemuck::Zeroable;
    type Uniform: bytemuck::Pod + bytemuck::Zeroable;

    fn verts(&self) -> &[Self::Vertex];
    fn indices(&self) -> &[u16];
    fn instances(&self) -> &[Self::Instance];
    fn uniform(&self) -> Self::Uniform;
    fn batches(&self) -> &[InstancedMarkBatch];
    fn texture_size(&self) -> Extent3d;

    fn shader(&self) -> &str;
    fn vertex_entry_point(&self) -> &str;
    fn fragment_entry_point(&self) -> &str;
    fn instance_desc(&self) -> wgpu::VertexBufferLayout<'static>;
    fn vertex_desc(&self) -> wgpu::VertexBufferLayout<'static>;

    fn mag_filter(&self) -> wgpu::FilterMode {
        wgpu::FilterMode::Nearest
    }
    fn min_filter(&self) -> wgpu::FilterMode {
        wgpu::FilterMode::Nearest
    }
    fn mipmap_filter(&self) -> wgpu::FilterMode {
        wgpu::FilterMode::Nearest
    }
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MarkUniform {
    pub adjustment_scale: [f32; 2],
    pub adjustment_offset: [f32; 2],
}

pub struct InstancedMarkRenderer {
    pub render_pipeline: wgpu::RenderPipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
    instance_buffers: Vec<InstancedInstanceBuffer>,
    pub uniform_bind_group: wgpu::BindGroup,
    pub texture: wgpu::Texture,
    pub texture_size: wgpu::Extent3d,
    pub texture_bind_group: wgpu::BindGroup,
    pub clip: Clip,
    pub scale: f32,
    pub mark_uniform_buffer: wgpu::Buffer,
}

struct InstancedInstanceBuffer {
    buffer: wgpu::Buffer,
    batches: Vec<InstancedMarkBatch>,
}

impl InstancedMarkRenderer {
    pub fn new<I, V, U>(
        device: &Device,
        texture_format: TextureFormat,
        sample_count: u32,
        mark_shader: Box<dyn InstancedMarkShader<Instance = I, Vertex = V, Uniform = U>>,
        clip: Clip,
        scale: f32,
    ) -> Self
    where
        I: bytemuck::Pod + bytemuck::Zeroable,
        V: bytemuck::Pod + bytemuck::Zeroable,
        U: bytemuck::Pod + bytemuck::Zeroable,
    {
        // Uniforms
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[mark_shader.uniform()]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create mark uniform buffer with initial values
        let mark_uniform = MarkUniform {
            adjustment_scale: [1.0, 1.0],
            adjustment_offset: [0.0, 0.0],
        };

        let mark_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mark Uniform Buffer"),
            contents: bytemuck::cast_slice(&[mark_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("chart_uniform_layout"),
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &uniform_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: mark_uniform_buffer.as_entire_binding(),
                },
            ],
            label: Some("uniform_bind_group"),
        });

        // Create Texture
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: mark_shader.texture_size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: Some("diffuse_texture"),
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: mark_shader.mag_filter(),
            min_filter: mark_shader.min_filter(),
            mipmap_filter: mark_shader.mipmap_filter(),
            ..Default::default()
        });

        // Create texture/sampler bind grous
        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        // This should match the filterable field of the
                        // corresponding Texture entry above.
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
            label: Some("texture_bind_group"),
        });

        // Shaders
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(mark_shader.shader().into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[
                    &uniform_layout,            // group(0) - contains both uniforms
                    &texture_bind_group_layout, // group(1) - for texture
                ],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(mark_shader.vertex_entry_point()),
                compilation_options: Default::default(),
                buffers: &[mark_shader.vertex_desc(), mark_shader.instance_desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(mark_shader.fragment_entry_point()),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: texture_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(mark_shader.verts()),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(mark_shader.indices()),
            usage: wgpu::BufferUsages::INDEX,
        });
        let num_indices = mark_shader.indices().len() as u32;

        let instance_buffers =
            make_instance_buffers(device, mark_shader.instances(), mark_shader.batches());

        Self {
            render_pipeline,
            vertex_buffer,
            index_buffer,
            num_indices,
            instance_buffers,
            uniform_bind_group,
            texture,
            texture_size: mark_shader.texture_size(),
            texture_bind_group,
            clip,
            scale,
            mark_uniform_buffer,
        }
    }

    pub fn render(
        &self,
        device: &Device,
        render_target_extent: Extent3d,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) -> CommandBuffer {
        let mut mark_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Mark Render Encoder"),
        });
        self.encode_into(
            device,
            &mut mark_encoder,
            render_target_extent,
            texture_view,
            resolve_target,
            x_adjustment,
            y_adjustment,
        );
        mark_encoder.finish()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode_into(
        &self,
        device: &Device,
        mark_encoder: &mut wgpu::CommandEncoder,
        render_target_extent: Extent3d,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        let timing_enabled =
            tracing::enabled!(target: "avenger_wgpu::render_breakdown", tracing::Level::DEBUG);
        let total_start = timing_enabled.then(Instant::now);
        let mut checkpoint = total_start;

        // Update mark uniforms
        let adjustment_scale = [
            x_adjustment.map(|a| a.scale).unwrap_or(1.0),
            y_adjustment.map(|a| a.scale).unwrap_or(1.0),
        ];
        let adjustment_offset = [
            x_adjustment.map(|a| a.offset).unwrap_or(0.0),
            y_adjustment.map(|a| a.offset).unwrap_or(0.0),
        ];
        let mark_uniform = MarkUniform {
            adjustment_scale,
            adjustment_offset,
        };

        let temp_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Temp Mark Uniform Buffer"),
            contents: bytemuck::cast_slice(&[mark_uniform]),
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        let uniform_buffer_us = checkpoint_us(&mut checkpoint);

        mark_encoder.copy_buffer_to_buffer(
            &temp_buffer,
            0,
            &self.mark_uniform_buffer,
            0,
            std::mem::size_of::<MarkUniform>() as u64,
        );
        let uniform_copy_us = checkpoint_us(&mut checkpoint);

        for instance_buffer in &self.instance_buffers {
            for batch in &instance_buffer.batches {
                if let Some(img) = &batch.image {
                    let temp_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Temp Buffer"),
                            contents: img.to_rgba8().as_raw(),
                            usage: wgpu::BufferUsages::COPY_SRC,
                        });
                    mark_encoder.copy_buffer_to_texture(
                        wgpu::TexelCopyBufferInfo {
                            buffer: &temp_buffer,
                            layout: TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(4 * self.texture_size.width),
                                rows_per_image: Some(self.texture_size.height),
                            },
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &self.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        self.texture_size,
                    );
                }

                {
                    let mut render_pass =
                        mark_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Mark Render Pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: texture_view,
                                resolve_target,
                                depth_slice: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            occlusion_query_set: None,
                            timestamp_writes: None,
                        });

                    render_pass.set_pipeline(&self.render_pipeline);
                    render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                    render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    render_pass.set_vertex_buffer(1, instance_buffer.buffer.slice(..));
                    render_pass
                        .set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

                    if let Clip::Rect {
                        x,
                        y,
                        width,
                        height,
                    } = self.clip
                    {
                        let px = (x * self.scale) as u32;
                        let py = (y * self.scale) as u32;
                        let pw = (width * self.scale) as u32;
                        let ph = (height * self.scale) as u32;
                        let cx = px.min(render_target_extent.width);
                        let cy = py.min(render_target_extent.height);
                        render_pass.set_scissor_rect(
                            cx,
                            cy,
                            pw.min(render_target_extent.width - cx),
                            ph.min(render_target_extent.height - cy),
                        );
                    }

                    render_pass.draw_indexed(0..self.num_indices, 0, batch.instances_range.clone());
                }
            }
        }
        let encode_us = checkpoint_us(&mut checkpoint);

        if let Some(start) = total_start {
            let instance_count: u32 = self
                .instance_buffers
                .iter()
                .flat_map(|chunk| chunk.batches.iter())
                .map(|batch| batch.instances_range.end - batch.instances_range.start)
                .sum();
            let image_batch_count = self
                .instance_buffers
                .iter()
                .flat_map(|chunk| chunk.batches.iter())
                .filter(|batch| batch.image.is_some())
                .count();
            tracing::debug!(
                target: "avenger_wgpu::render_breakdown",
                renderer = "instanced",
                total_ms = start.elapsed().as_secs_f64() * 1000.0,
                uniform_buffer_ms = us_to_ms(uniform_buffer_us),
                uniform_copy_ms = us_to_ms(uniform_copy_us),
                encode_ms = us_to_ms(encode_us),
                buffer_count = self.instance_buffers.len(),
                batch_count = self
                    .instance_buffers
                    .iter()
                    .map(|chunk| chunk.batches.len())
                    .sum::<usize>(),
                image_batch_count,
                instance_count,
                index_count = self.num_indices,
                "wgpu.render.renderer"
            );
        }
    }
}

fn make_instance_buffers<I>(
    device: &Device,
    instances: &[I],
    batches: &[InstancedMarkBatch],
) -> Vec<InstancedInstanceBuffer>
where
    I: bytemuck::Pod + bytemuck::Zeroable,
{
    let instance_size = std::mem::size_of::<I>().max(1);
    let max_buffer_size = device.limits().max_buffer_size as usize;
    let max_instances_per_buffer = (max_buffer_size / instance_size).max(1);
    let mut chunks = Vec::new();

    for chunk_start in (0..instances.len()).step_by(max_instances_per_buffer) {
        let chunk_end = (chunk_start + max_instances_per_buffer).min(instances.len());
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Instance Buffer"),
            contents: bytemuck::cast_slice(&instances[chunk_start..chunk_end]),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let mut chunk_batches = Vec::new();
        for batch in batches {
            let batch_start = batch.instances_range.start as usize;
            let batch_end = batch.instances_range.end as usize;
            let overlap_start = batch_start.max(chunk_start);
            let overlap_end = batch_end.min(chunk_end);
            if overlap_start < overlap_end {
                chunk_batches.push(InstancedMarkBatch {
                    instances_range: (overlap_start - chunk_start) as u32
                        ..(overlap_end - chunk_start) as u32,
                    image: batch.image.clone(),
                });
            }
        }

        chunks.push(InstancedInstanceBuffer {
            buffer,
            batches: chunk_batches,
        });
    }

    chunks
}

fn checkpoint_us(checkpoint: &mut Option<Instant>) -> u64 {
    if let Some(previous) = checkpoint {
        let now = Instant::now();
        let elapsed_us = now.duration_since(*previous).as_micros() as u64;
        *previous = now;
        elapsed_us
    } else {
        0
    }
}

fn us_to_ms(us: u64) -> f64 {
    us as f64 / 1000.0
}

pub trait InstancedMarkFingerprint {
    /// Fingerprint that identifies the mark content used by an instance renderer.
    ///
    /// A renderer cache must combine this fingerprint with any immutable render context
    /// stored in uniforms, such as the mark origin, canvas dimensions, and effective clip.
    fn instanced_fingerprint(&self) -> u64;
}
