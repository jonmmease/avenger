use std::ops::Range;
use wgpu::util::DeviceExt;
use wgpu::{CommandBuffer, Device, Extent3d, ImageDataLayout, TextureFormat, TextureView};

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

pub struct InstancedMarkRenderer {
    pub render_pipeline: wgpu::RenderPipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
    pub instance_buffer: wgpu::Buffer,
    pub batches: Vec<InstancedMarkBatch>,
    pub uniform_bind_group: wgpu::BindGroup,
    pub texture: wgpu::Texture,
    pub texture_size: wgpu::Extent3d,
    pub texture_bind_group: wgpu::BindGroup,
}

impl InstancedMarkRenderer {
    pub fn new<I, V, U>(
        device: &Device,
        texture_format: TextureFormat,
        sample_count: u32,
        mark_shader: Box<dyn InstancedMarkShader<Instance = I, Vertex = V, Uniform = U>>,
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

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
            label: Some("chart_uniform_layout"),
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
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
                bind_group_layouts: &[&uniform_layout, &texture_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: mark_shader.vertex_entry_point(),
                buffers: &[mark_shader.vertex_desc(), mark_shader.instance_desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: mark_shader.fragment_entry_point(),
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

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Instance Buffer"),
            contents: bytemuck::cast_slice(mark_shader.instances()),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            render_pipeline,
            vertex_buffer,
            index_buffer,
            batches: Vec::from(mark_shader.batches()),
            num_indices,
            instance_buffer,
            uniform_bind_group,
            texture,
            texture_size: mark_shader.texture_size(),
            texture_bind_group,
        }
    }

    pub fn render(
        &self,
        device: &Device,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
    ) -> CommandBuffer {
        let mut mark_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Mark Render Encoder"),
        });

        for batch in self.batches.iter() {
            if let Some(img) = &batch.image {
                let temp_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Temp Buffer"),
                    contents: img.to_rgba8().as_raw(),
                    usage: wgpu::BufferUsages::COPY_SRC,
                });
                mark_encoder.copy_buffer_to_texture(
                    wgpu::ImageCopyBuffer {
                        buffer: &temp_buffer,
                        layout: ImageDataLayout {
                            offset: 0,
                            bytes_per_row: Some(4 * self.texture_size.width),
                            rows_per_image: Some(self.texture_size.height),
                        },
                    },
                    wgpu::ImageCopyTexture {
                        texture: &self.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    self.texture_size,
                );
            }

            {
                let mut render_pass = mark_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Mark Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: texture_view,
                        resolve_target,
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
                render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                render_pass
                    .set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.num_indices, 0, batch.instances_range.clone());
            }
        }

        mark_encoder.finish()
    }
}
