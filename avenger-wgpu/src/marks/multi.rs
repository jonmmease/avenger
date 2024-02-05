use crate::canvas::CanvasDimensions;
use crate::error::AvengerWgpuError;
use crate::marks::gradient::to_color_or_gradient_coord;
use avenger::marks::group::GroupBounds;
use avenger::marks::line::LineMark;
use avenger::marks::path::PathMark;
use avenger::marks::value::{Gradient, StrokeCap, StrokeJoin};
use image::DynamicImage;
use itertools::izip;
use lyon::algorithms::aabb::bounding_box;
use lyon::geom::euclid::Vector2D;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, LineCap,
    LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex, StrokeVertexConstructor,
    VertexBuffers,
};
use std::ops::Range;
use wgpu::util::DeviceExt;
use wgpu::{
    BindGroup, BindGroupLayout, CommandBuffer, Device, Extent3d, Queue, TextureFormat, TextureView,
    VertexBufferLayout,
};

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MultiUniform {
    pub size: [f32; 2],
    pub scale: f32,
    _pad: [f32; 1],
}

#[derive(Clone, Copy)]
pub struct ClipRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MultiVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub top_left: [f32; 2],
    pub bottom_right: [f32; 2],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
    1 => Float32x4,     // color
    2 => Float32x2,     // top_left
    3 => Float32x2,     // bottom_right
];

impl MultiVertex {
    pub fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<MultiVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }
    }
}

#[derive(Clone)]
pub struct MultiMarkBatch {
    pub indices_range: Range<u32>,
    pub clip: Option<ClipRect>,
    pub image_atlas_index: Option<usize>,
    pub gradient_atlas_index: Option<usize>,
}

pub struct MultiMarkRenderer {
    verts_inds: Vec<(Vec<MultiVertex>, Vec<u32>)>,
    gradient_atlases: Vec<DynamicImage>,
    image_atlases: Vec<DynamicImage>,
    batches: Vec<MultiMarkBatch>,
    uniform: MultiUniform,

    gradient_size: Extent3d,
    image_size: Extent3d,
    dimensions: CanvasDimensions,
}

impl MultiMarkRenderer {
    pub fn new(dimensions: CanvasDimensions) -> Self {
        Self {
            verts_inds: vec![],
            gradient_atlases: vec![],
            image_atlases: vec![],
            batches: vec![],
            dimensions,
            uniform: MultiUniform {
                size: dimensions.size,
                scale: dimensions.scale,
                _pad: [0.0],
            },
            gradient_size: Extent3d {
                width: 256,
                height: 128,
                depth_or_array_layers: 1,
            },
            image_size: Extent3d {
                width: 2048,
                height: 2048,
                depth_or_array_layers: 1,
            },
        }
    }

    fn register_gradients(&mut self, gradients: &[Gradient]) -> (Option<usize>, Extent3d, u32) {
        // Add gradients to image, return offset to apply to indices
        // todo!()
        (None, Extent3d::default(), 0)
    }

    pub fn add_path_mark(
        &mut self,
        mark: &PathMark,
        bounds: GroupBounds,
    ) -> Result<(), AvengerWgpuError> {
        let (gradient_atlas_index, grad_extent, grad_offset) =
            self.register_gradients(&mark.gradients);

        let verts_inds = izip!(
            mark.path_iter(),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.transform_iter(),
        ).map(|(path, fill, stroke, transform)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
            // Apply transform to path
            let path = path.clone().transformed(
                &transform.then_translate(Vector2D::new(bounds.x, bounds.y))
            );
            let bbox = bounding_box(&path);

            // Create vertex/index buffer builder
            let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
            let mut builder = BuffersBuilder::new(
                &mut buffers,
                VertexPositions {
                    fill: to_color_or_gradient_coord(fill, grad_extent),
                    stroke: to_color_or_gradient_coord(stroke, grad_extent),
                    top_left: bbox.min.to_array(),
                    bottom_right: bbox.max.to_array(),
                },
            );

            // Tesselate fill
            let mut fill_tessellator = FillTessellator::new();
            let fill_options = FillOptions::default().with_tolerance(0.05);

            fill_tessellator.tessellate_path(&path, &fill_options, &mut builder)?;

            // Tesselate stroke
            if let Some(stroke_width) = mark.stroke_width {
                let mut stroke_tessellator = StrokeTessellator::new();
                let stroke_options = StrokeOptions::default()
                    .with_tolerance(0.05)
                    .with_line_join(match mark.stroke_join {
                        StrokeJoin::Miter => LineJoin::Miter,
                        StrokeJoin::Round => LineJoin::Round,
                        StrokeJoin::Bevel => LineJoin::Bevel,
                    })
                    .with_line_cap(match mark.stroke_cap {
                        StrokeCap::Butt => LineCap::Butt,
                        StrokeCap::Round => LineCap::Round,
                        StrokeCap::Square => LineCap::Square,
                    })
                    .with_line_width(stroke_width);
                stroke_tessellator.tessellate_path(&path, &stroke_options, &mut builder)?;
            }

            Ok((buffers.vertices, buffers.indices))
        }).collect::<Result<Vec<_>, AvengerWgpuError>>()?;

        let start_ind = self.num_indices();
        let inds_len: usize = verts_inds.iter().map(|(_, i)| i.len()).sum();
        let indices_range = (start_ind as u32)..((start_ind + inds_len) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: None,
            image_atlas_index: None,
            gradient_atlas_index,
        };

        self.verts_inds.extend(verts_inds);
        self.batches.push(batch);
        Ok(())
    }

    pub fn add_line_mark(
        &mut self,
        mark: &LineMark,
        bounds: GroupBounds,
    ) -> Result<(), AvengerWgpuError> {
        todo!()
    }

    fn num_indices(&self) -> usize {
        self.verts_inds.iter().map(|(_, inds)| inds.len()).sum()
    }

    fn num_verticies(&self) -> usize {
        self.verts_inds.iter().map(|(v, _)| v.len()).sum()
    }

    pub fn render(
        &self,
        device: &Device,
        queue: &Queue,
        texture_format: TextureFormat,
        sample_count: u32,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
    ) -> CommandBuffer {
        // Uniforms
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Multi Uniform Buffer"),
            contents: bytemuck::cast_slice(&[self.uniform]),
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

        // Gradient Textures
        let (gradient_layout, gradient_texture_bind_groups) =
            Self::make_texture_bind_groups(device, queue, self.gradient_atlases.as_slice());

        // Image Textures
        let (image_layout, image_texture_bind_groups) =
            Self::make_texture_bind_groups(device, queue, self.image_atlases.as_slice());

        // Shaders
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("multi.wgsl").into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&uniform_layout, &gradient_layout, &image_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[MultiVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
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

        // flatten verts and inds
        let num_verts: usize = self.verts_inds.iter().map(|(v, _)| v.len()).sum();
        let num_inds: usize = self.verts_inds.iter().map(|(_, inds)| inds.len()).sum();
        let mut verticies: Vec<MultiVertex> = Vec::with_capacity(num_verts);
        let mut indices: Vec<u32> = Vec::with_capacity(num_inds);

        for (vs, inds) in &self.verts_inds {
            let offset = verticies.len() as u32;
            indices.extend(inds.iter().map(|i| *i + offset));
            verticies.extend(vs);
        }

        // Create vertex and index buffers
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(verticies.as_slice()),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(indices.as_slice()),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Create command encoder for marks
        let mut mark_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Multi Mark Render Encoder"),
        });

        // Render batches
        {
            let mut render_pass = mark_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Multi Mark Render Pass"),
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

            render_pass.set_pipeline(&render_pipeline);
            render_pass.set_bind_group(0, &uniform_bind_group, &[]);
            let mut last_grad_ind = 0;
            let mut last_img_ind = 0;
            render_pass.set_bind_group(1, &gradient_texture_bind_groups[last_grad_ind], &[]);
            render_pass.set_bind_group(2, &image_texture_bind_groups[last_img_ind], &[]);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);

            // Initialze textures with first entry
            for batch in &self.batches {
                // Update clip
                if let Some(clip) = batch.clip {
                    render_pass.set_scissor_rect(clip.x, clip.y, clip.width, clip.height);
                } else {
                    render_pass.set_scissor_rect(
                        0,
                        0,
                        self.dimensions.to_physical_width(),
                        self.dimensions.to_physical_height(),
                    );
                }

                // Update bind groups
                if let Some(grad_ind) = batch.gradient_atlas_index {
                    if grad_ind != last_grad_ind {
                        render_pass.set_bind_group(1, &gradient_texture_bind_groups[grad_ind], &[]);
                    }
                    last_grad_ind = grad_ind;
                }
                if let Some(img_ind) = batch.image_atlas_index {
                    if img_ind != last_img_ind {
                        render_pass.set_bind_group(2, &image_texture_bind_groups[img_ind], &[]);
                    }
                    last_img_ind = img_ind;
                }

                // draw inds
                render_pass.draw_indexed(batch.indices_range.clone(), 0, 0..1);
            }
        }

        mark_encoder.finish()
    }

    fn make_texture_bind_groups(
        device: &Device,
        queue: &Queue,
        images: &[DynamicImage],
    ) -> (BindGroupLayout, Vec<BindGroup>) {
        // Create texture for each image
        let mut texture_bind_groups: Vec<BindGroup> = Vec::new();

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

        if images.is_empty() {
            // If there are no images, create bind group with 1x1 texture so that there's something to bind to
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
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
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
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
                label: Some("empty texture_bind_group"),
            });
            texture_bind_groups.push(texture_bind_group);
        } else {
            for image in images {
                // Create Texture
                let size = Extent3d {
                    width: image.width(),
                    height: image.height(),
                    depth_or_array_layers: 1,
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    size,
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
                    mag_filter: wgpu::FilterMode::Nearest,
                    min_filter: wgpu::FilterMode::Nearest,
                    mipmap_filter: wgpu::FilterMode::Nearest,
                    ..Default::default()
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

                queue.write_texture(
                    // Tells wgpu where to copy the pixel data
                    wgpu::ImageCopyTexture {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    // The actual pixel data
                    image.to_rgba8().as_raw(),
                    // The layout of the texture
                    wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * image.width()),
                        rows_per_image: Some(image.height()),
                    },
                    size,
                );

                texture_bind_groups.push(texture_bind_group);
            }
        }
        (texture_bind_group_layout, texture_bind_groups)
    }
}

pub struct VertexPositions {
    fill: [f32; 4],
    stroke: [f32; 4],
    top_left: [f32; 2],
    bottom_right: [f32; 2],
}

impl FillVertexConstructor<MultiVertex> for VertexPositions {
    fn new_vertex(&mut self, vertex: FillVertex) -> MultiVertex {
        MultiVertex {
            position: [vertex.position().x, vertex.position().y],
            color: self.fill,
            top_left: self.top_left,
            bottom_right: self.bottom_right,
        }
    }
}

impl StrokeVertexConstructor<MultiVertex> for VertexPositions {
    fn new_vertex(&mut self, vertex: StrokeVertex) -> MultiVertex {
        MultiVertex {
            position: [vertex.position().x, vertex.position().y],
            color: self.stroke,
            top_left: self.top_left,
            bottom_right: self.bottom_right,
        }
    }
}