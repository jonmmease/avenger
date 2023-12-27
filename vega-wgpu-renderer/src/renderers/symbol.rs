use crate::renderers::canvas::CanvasUniform;
use crate::renderers::vertex::Vertex;
use crate::scene::symbol::SymbolInstance;
use crate::specs::symbol::SymbolItemSpec;
use wgpu::util::DeviceExt;
use wgpu::{CommandBuffer, Device, Surface, TextureFormat, TextureView};

fn symbol_verts() -> Vec<Vertex> {
    let tan30: f32 = (30.0 * std::f32::consts::PI / 180.0).tan();
    let radius: f32 = (1.0 / (2.0 * tan30)).sqrt();

    vec![
        Vertex {
            position: [0.0, -radius, 0.0],
        },
        Vertex {
            position: [radius, 0.0, 0.0],
        },
        Vertex {
            position: [0.0, radius, 0.0],
        },
        Vertex {
            position: [-radius, 0.0, 0.0],
        },
    ]
}

const SYMBOL_INDICES: &[u16] = &[0, 1, 2, 0, 2, 3];

pub struct SymbolMarkRenderer {
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    num_vertices: u32,
    index_buffer: wgpu::Buffer,
    num_indices: u32,
    instance_buffer: wgpu::Buffer,
    num_instances: u32,
    uniform_buffer: wgpu::Buffer,
    uniform: CanvasUniform,
    uniform_bind_group: wgpu::BindGroup,
}

impl SymbolMarkRenderer {
    pub fn new(
        device: &Device,
        uniform: CanvasUniform,
        texture_format: TextureFormat,
        instances: &[SymbolInstance],
    ) -> Self {
        // Uniforms
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
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

        let _render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&uniform_layout],
                push_constant_ranges: &[],
            });

        // Shaders
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("symbol.wgsl").into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&uniform_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[Vertex::desc(), SymbolInstance::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: texture_format,
                    blend: Some(wgpu::BlendState::REPLACE),
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
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        let verts = symbol_verts();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(verts.as_slice()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let num_vertices = verts.len() as u32;

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(SYMBOL_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });
        let num_indices = SYMBOL_INDICES.len() as u32;

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Instance Buffer"),
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let num_instances = instances.len() as u32;

        Self {
            render_pipeline,
            vertex_buffer,
            num_vertices,
            index_buffer,
            num_indices,
            instance_buffer,
            num_instances,
            uniform_buffer,
            uniform,
            uniform_bind_group,
        }
    }

    pub fn render(&self, device: &Device, texture_view: &TextureView) -> CommandBuffer {
        let mut mark_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Mark Render Encoder"),
        });

        {
            let mut render_pass = mark_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Mark Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: texture_view,
                    resolve_target: None,
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
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..self.num_indices, 0, 0..self.num_instances);
        }

        return mark_encoder.finish();
    }
}
