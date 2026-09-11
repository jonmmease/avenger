use std::ops::{Mul, Range};

use avenger_color::ColorOrGradient;
use avenger_common::{
    canvas::CanvasDimensions,
    time::Instant,
    types::{PathTransform, StrokeCap, StrokeJoin},
};
use avenger_scenegraph::marks::{
    arc::SceneArcMark,
    area::SceneAreaMark,
    group::Clip,
    image::SceneImageMark,
    line::SceneLineMark,
    path::ScenePathMark,
    rect::SceneRectMark,
    rule::SceneRuleMark,
    stroke_dash::dash_paths,
    symbol::SceneSymbolMark,
    text_leader::{TextLeaderArrowhead, TextLeaderGeometry, TextLeaderPath},
    trail::SceneTrailMark,
};
use etagere::euclid::UnknownUnit;
use image::DynamicImage;
use itertools::izip;
use lyon::{
    algorithms::aabb::bounding_box,
    geom::{
        euclid::{Point2D, Vector2D},
        Angle, Box2D,
    },
    lyon_tessellation::{
        BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, LineCap,
        LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex, StrokeVertexConstructor,
        VertexBuffers,
    },
    path::{builder::BorderRadii, geom::point, Path, Winding},
};
use wgpu::{
    util::DeviceExt, BindGroup, BindGroupLayout, CommandBuffer, Device, Extent3d, Queue,
    RenderPipeline, ShaderModule, TextureFormat, TextureView, VertexBufferLayout,
};

use crate::{
    error::AvengerWgpuError,
    marks::{
        gradient::{to_color_or_gradient_coord, GradientAtlasBuilder},
        image::ImageAtlasBuilder,
    },
};

#[cfg(feature = "rayon")]
use {crate::par_izip, rayon::prelude::*};

pub const GRADIENT_TEXTURE_CODE: f32 = -1.0;
pub const IMAGE_TEXTURE_CODE: f32 = -2.0;
pub const TEXT_TEXTURE_CODE: f32 = -3.0;
pub const TEXT_TEXTURE_NEAREST_CODE: f32 = -4.0;

const NORMALIZED_SYMBOL_STROKE_WIDTH: f32 = 0.1;
const STENCIL_ATTACHMENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;

pub(crate) fn is_axis_aligned_angle(angle: f32) -> bool {
    let normalized = angle.rem_euclid(360.0);
    (normalized < 0.001)
        || ((normalized - 90.0).abs() < 0.001)
        || ((normalized - 180.0).abs() < 0.001)
        || ((normalized - 270.0).abs() < 0.001)
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MultiUniform {
    pub size: [f32; 2],
    pub scale: f32,
    _pad: [f32; 1],
    pub translation: [f32; 2],
    _pad2: [f32; 2],
}

#[allow(dead_code)]
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
    pub clip: Clip,
    pub clip_indices_range: Option<Range<u32>>,
    pub image_atlas_index: Option<usize>,
    pub image_smooth: bool,
    pub gradient_atlas_index: Option<usize>,
    pub text_atlas_index: Option<usize>,
}

/// Per-frame GPU resources for a `MultiMarkRenderer`, built once by `prepare()` and
/// reused across any number of `encode_multi_ranges` calls. Splitting prepare from
/// encode lets one shared renderer's geometry/atlas/uniform be uploaded a single
/// time per frame while individual z-runs (batch ranges) are encoded separately and
/// interleaved with instanced marks.
pub(crate) struct PreparedMulti {
    uniform_bind_group: BindGroup,
    gradient_texture_bind_groups: Vec<BindGroup>,
    image_texture_bind_groups: Vec<BindGroup>,
    image_texture_bind_groups_nearest: Vec<BindGroup>,
    stencil_buffer: Option<wgpu::Texture>,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    clip_vertex_buffer: wgpu::Buffer,
    clip_index_buffer: wgpu::Buffer,
}

pub struct MultiMarkRenderer {
    verts_inds: Vec<(Vec<MultiVertex>, Vec<u32>)>,
    clip_verts_inds: Vec<(Vec<MultiVertex>, Vec<u32>)>,
    batches: Vec<MultiMarkBatch>,
    uniform: MultiUniform,
    gradient_atlas_builder: GradientAtlasBuilder,
    image_atlas_builder: ImageAtlasBuilder,
    dimensions: CanvasDimensions,
}

pub(crate) struct TextLeaderRenderItem {
    pub geometry: TextLeaderGeometry,
    pub stroke: ColorOrGradient,
    pub stroke_width: f32,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_dash: Option<Vec<f32>>,
}

#[derive(Clone)]
pub struct MultiMarkRenderResources {
    uniform_layout: BindGroupLayout,
    texture_layout: BindGroupLayout,
    text_layout: BindGroupLayout,
    render_pipeline: RenderPipeline,
    stencil_render_pipeline: RenderPipeline,
    stencil_pipeline: RenderPipeline,
    sample_count: u32,
}

impl MultiMarkRenderResources {
    pub fn new(device: &Device, texture_format: TextureFormat, sample_count: u32) -> Self {
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
        let texture_layout = Self::make_texture_bind_group_layout(device);
        let text_layout = Self::make_text_bind_group_layout(device);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("multi.wgsl").into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[
                    &uniform_layout,
                    &texture_layout,
                    &texture_layout,
                    &text_layout,
                ],
                push_constant_ranges: &[],
            });
        let render_pipeline = Self::make_render_pipeline(
            device,
            texture_format,
            sample_count,
            &render_pipeline_layout,
            &shader,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::ColorWrites::ALL,
            wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
        );
        // Tile-array variant: same vertex layout and blend state, group 2
        // swapped to a texture_2d_array layout. Groups 0/1/3 share the
        // same BindGroupLayout objects as the main pipeline, so bind
        // groups stay compatible across mid-pass pipeline switches.

        let stencil_equal_face = wgpu::StencilFaceState {
            compare: wgpu::CompareFunction::Equal,
            ..Default::default()
        };
        let stencil_replace_face = wgpu::StencilFaceState {
            compare: wgpu::CompareFunction::Always,
            pass_op: wgpu::StencilOperation::Replace,
            ..Default::default()
        };
        let stencil_render_pipeline = Self::make_render_pipeline(
            device,
            texture_format,
            sample_count,
            &render_pipeline_layout,
            &shader,
            Some(wgpu::DepthStencilState {
                format: STENCIL_ATTACHMENT_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState {
                    front: stencil_equal_face,
                    back: stencil_equal_face,
                    read_mask: !0,
                    write_mask: !0,
                },
                bias: Default::default(),
            }),
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::ColorWrites::ALL,
            wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
        );
        let stencil_pipeline = Self::make_render_pipeline(
            device,
            texture_format,
            sample_count,
            &render_pipeline_layout,
            &shader,
            Some(wgpu::DepthStencilState {
                format: STENCIL_ATTACHMENT_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState {
                    front: stencil_replace_face,
                    back: stencil_replace_face,
                    read_mask: !0,
                    write_mask: !0,
                },
                bias: Default::default(),
            }),
            None,
            wgpu::ColorWrites::empty(),
            Default::default(),
        );

        Self {
            uniform_layout,
            texture_layout,
            text_layout,
            render_pipeline,
            stencil_render_pipeline,
            stencil_pipeline,
            sample_count,
        }
    }

    fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// The bind-group layout for the (dual-sampler) text atlas pages. Exposed so the
    /// canvas can build the shared text bind groups once per frame.
    pub(crate) fn text_layout(&self) -> &BindGroupLayout {
        &self.text_layout
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
    )]
    fn make_render_pipeline(
        device: &Device,
        texture_format: TextureFormat,
        sample_count: u32,
        render_pipeline_layout: &wgpu::PipelineLayout,
        shader: &ShaderModule,
        depth_stencil: Option<wgpu::DepthStencilState>,
        blend: Option<wgpu::BlendState>,
        write_mask: wgpu::ColorWrites,
        primitive: wgpu::PrimitiveState,
    ) -> RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[MultiVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: texture_format,
                    blend,
                    write_mask,
                })],
            }),
            primitive,
            depth_stencil,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        })
    }

    fn make_texture_bind_group_layout(device: &Device) -> BindGroupLayout {
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
            label: Some("texture_bind_group_layout"),
        })
    }

    fn make_text_bind_group_layout(device: &Device) -> BindGroupLayout {
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
            label: Some("text_dual_sampler_bind_group_layout"),
        })
    }
}

impl MultiMarkRenderer {
    #[tracing::instrument(skip_all)]
    pub fn add_image_mark(
        &mut self,
        mark: &SceneImageMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        let verts_inds = izip!(mark.image_iter(), mark.transformed_path_iter(origin))
            .map(
                |(img, path)| -> Result<(usize, Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                    let Some(rgba_image) = img.to_image() else {
                        return Err(AvengerWgpuError::ConversionError(
                            "Failed to convert raw image to rgba image".to_string(),
                        ));
                    };

                    let (atlas_index, tex_coords) =
                        self.image_atlas_builder.register_image(&rgba_image)?;

                    // Get bounding box of path
                    let bbox = bounding_box(&path);
                    let left = bbox.min.x;
                    let top = bbox.min.y;
                    let width = bbox.max.x - bbox.min.x;
                    let height = bbox.max.y - bbox.min.y;

                    let top_left = [top, left];
                    let bottom_right = [top + height, left + width];
                    let verts = vec![
                        // Upper left
                        MultiVertex {
                            color: [IMAGE_TEXTURE_CODE, tex_coords.x0, tex_coords.y0, 0.0],
                            position: [left, top],
                            top_left,
                            bottom_right,
                        },
                        // Lower left
                        MultiVertex {
                            color: [IMAGE_TEXTURE_CODE, tex_coords.x0, tex_coords.y1, 0.0],
                            position: [left, top + height],
                            top_left,
                            bottom_right,
                        },
                        // Lower right
                        MultiVertex {
                            color: [IMAGE_TEXTURE_CODE, tex_coords.x1, tex_coords.y1, 0.0],
                            position: [left + width, top + height],
                            top_left,
                            bottom_right,
                        },
                        // Upper right
                        MultiVertex {
                            color: [IMAGE_TEXTURE_CODE, tex_coords.x1, tex_coords.y0, 0.0],
                            position: [left + width, top],
                            top_left,
                            bottom_right,
                        },
                    ];
                    let indices: Vec<u32> = vec![0, 1, 2, 0, 2, 3];
                    Ok((atlas_index, verts, indices))
                },
            )
            .collect::<Result<Vec<_>, AvengerWgpuError>>()?;

        // Construct batches, one batch per image atlas index
        let start_ind = self.num_indices() as u32;
        let mut next_batch = MultiMarkBatch {
            indices_range: start_ind..start_ind,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_smooth: mark.smooth,
            image_atlas_index: None,
            gradient_atlas_index: None,
            text_atlas_index: None,
        };

        for (atlas_index, verts, inds) in verts_inds {
            if next_batch.image_atlas_index.unwrap_or(atlas_index) == atlas_index {
                // update next batch with atlas index and inds range
                next_batch.image_atlas_index = Some(atlas_index);
                next_batch.indices_range = next_batch.indices_range.start
                    ..(next_batch.indices_range.end + inds.len() as u32);
            } else {
                // create new batch
                let start_ind = next_batch.indices_range.end;
                // Initialize new next_batch and swap to avoid extra mem copy
                let mut full_batch = MultiMarkBatch {
                    indices_range: start_ind..(start_ind + inds.len() as u32),
                    clip: clip.maybe_clip(mark.clip),
                    clip_indices_range: self.add_clip_path(clip, mark.clip)?,
                    image_smooth: mark.smooth,
                    image_atlas_index: Some(atlas_index),
                    gradient_atlas_index: None,
                    text_atlas_index: None,
                };
                std::mem::swap(&mut full_batch, &mut next_batch);
                self.batches.push(full_batch);
            }

            // Add verts and indices
            self.verts_inds.push((verts, inds))
        }

        self.batches.push(next_batch);
        Ok(())
    }

    pub fn new(dimensions: CanvasDimensions) -> Self {
        Self {
            verts_inds: vec![],
            clip_verts_inds: vec![],
            batches: vec![],
            dimensions,
            uniform: MultiUniform {
                size: dimensions.size,
                scale: dimensions.scale,
                _pad: [0.0],
                translation: [0.0, 0.0],
                _pad2: [0.0, 0.0],
            },
            gradient_atlas_builder: GradientAtlasBuilder::new(),
            image_atlas_builder: ImageAtlasBuilder::new(),
        }
    }

    pub fn clear(&mut self) {
        self.verts_inds.clear();
        self.clip_verts_inds.clear();
        self.batches.clear();
        self.gradient_atlas_builder = GradientAtlasBuilder::new();
        self.image_atlas_builder = ImageAtlasBuilder::new();
    }

    pub fn reset_for_frame(&mut self, dimensions: CanvasDimensions) {
        self.clear();
        self.dimensions = dimensions;
        self.uniform = MultiUniform {
            size: dimensions.size,
            scale: dimensions.scale,
            _pad: [0.0],
            translation: [0.0, 0.0],
            _pad2: [0.0, 0.0],
        };
    }

    /// Update the canvas dimensions without discarding accumulated geometry.
    /// Auxiliary cached renderers use this on resize.
    pub(crate) fn set_dimensions(&mut self, dimensions: CanvasDimensions) {
        self.dimensions = dimensions;
        self.uniform.size = dimensions.size;
        self.uniform.scale = dimensions.scale;
    }

    pub fn is_empty(&self) -> bool {
        self.verts_inds.is_empty() && self.clip_verts_inds.is_empty() && self.batches.is_empty()
    }

    /// Number of batches accumulated so far. Used by the canvas to record each
    /// z-run as a half-open batch range into this shared renderer.
    pub(crate) fn batch_count(&self) -> usize {
        self.batches.len()
    }

    fn add_clip_path(
        &mut self,
        clip: &Clip,
        should_clip: bool,
    ) -> Result<Option<Range<u32>>, AvengerWgpuError> {
        if !should_clip {
            return Ok(None);
        }

        if let Clip::Path(path) = &clip {
            // Tesselate path
            let bbox = bounding_box(path);

            // Create vertex/index buffer builder
            let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
            let mut builder = BuffersBuilder::new(
                &mut buffers,
                crate::marks::multi::VertexPositions {
                    fill: [0.0, 0.0, 0.0, 1.0],
                    stroke: [0.0, 0.0, 0.0, 0.0],
                    top_left: bbox.min.to_array(),
                    bottom_right: bbox.max.to_array(),
                },
            );

            // Tesselate fill
            let mut fill_tessellator = FillTessellator::new();
            let fill_options = FillOptions::default().with_tolerance(0.05);
            fill_tessellator.tessellate_path(path, &fill_options, &mut builder)?;

            let start_index = self.num_clip_indices() as u32;
            self.clip_verts_inds
                .push((buffers.vertices, buffers.indices));
            let end_index = self.num_clip_indices() as u32;
            Ok(Some(start_index..end_index))
        } else {
            Ok(None)
        }
    }

    #[tracing::instrument(skip_all)]
    pub fn add_rule_mark(
        &mut self,
        mark: &SceneRuleMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);

        let verts_inds = izip!(
            mark.transformed_path_iter(origin),
            mark.stroke_iter(),
            mark.stroke_width_iter(),
            mark.stroke_cap_iter()
        ).map(|(path, stroke, stroke_width, cap)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
            let bbox = bounding_box(&path);

            // Create vertex/index buffer builder
            let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
            let mut builder = BuffersBuilder::new(
                &mut buffers,
                VertexPositions {
                    fill: [0.0, 0.0, 0.0, 0.0],
                    stroke: to_color_or_gradient_coord(stroke, &grad_coords),
                    top_left: bbox.min.to_array(),
                    bottom_right: bbox.max.to_array(),
                },
            );

            // Tesselate stroke
            let mut stroke_tessellator = StrokeTessellator::new();
            let stroke_options = StrokeOptions::default()
                .with_tolerance(0.05)
                .with_line_join(LineJoin::Miter)
                .with_line_cap(match cap {
                    StrokeCap::Butt => LineCap::Butt,
                    StrokeCap::Round => LineCap::Round,
                    StrokeCap::Square => LineCap::Square,
                })
                .with_line_width(*stroke_width);
            stroke_tessellator.tessellate_path(&path, &stroke_options, &mut builder)?;
            Ok((buffers.vertices, buffers.indices))
        }).collect::<Result<Vec<_>, AvengerWgpuError>>()?;

        let start_ind = self.num_indices();
        let inds_len: usize = verts_inds.iter().map(|(_, i)| i.len()).sum();
        let indices_range = (start_ind as u32)..((start_ind + inds_len) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
            image_smooth: true,
            gradient_atlas_index,
            text_atlas_index: None,
        };

        self.verts_inds.extend(verts_inds);
        self.batches.push(batch);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub fn add_rect_mark(
        &mut self,
        mark: &SceneRectMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);

        let verts_inds = if mark.gradients.is_empty()
            && mark.stroke_width.equals_scalar(0.0)
            && mark.corner_radius.equals_scalar(0.0)
        {
            // Handle simple case of no stroke, rounded corners, or gradient. In this case we don't need
            // lyon to perform the tesselation, which saves a bit of time. The contents of this loop are so
            // fast that parallelization doesn't help.
            let mut verts: Vec<MultiVertex> = Vec::with_capacity((mark.len * 4) as usize);
            let mut indicies: Vec<u32> = Vec::with_capacity((mark.len * 6) as usize);

            for (i, x, y, x2, y2, fill) in izip!(
                0..mark.len,
                mark.x_iter(),
                mark.y_iter(),
                mark.x2_iter(),
                mark.y2_iter(),
                mark.fill_iter()
            ) {
                let x0 = f32::min(*x, x2) + origin[0];
                let x1 = f32::max(*x, x2) + origin[0];
                let y0 = f32::min(*y, y2) + origin[1];
                let y1 = f32::max(*y, y2) + origin[1];
                let top_left = [x0, y0];
                let bottom_right = [x1, y1];
                let color = fill.color_or_transparent();
                verts.push(MultiVertex {
                    position: [x0, y0],
                    color,
                    top_left,
                    bottom_right,
                });
                verts.push(MultiVertex {
                    position: [x0, y1],
                    color,
                    top_left,
                    bottom_right,
                });
                verts.push(MultiVertex {
                    position: [x1, y1],
                    color,
                    top_left,
                    bottom_right,
                });
                verts.push(MultiVertex {
                    position: [x1, y0],
                    color,
                    top_left,
                    bottom_right,
                });
                let offset = i * 4;
                indicies.extend([
                    offset,
                    offset + 1,
                    offset + 2,
                    offset,
                    offset + 2,
                    offset + 3,
                ])
            }

            vec![(verts, indicies)]
        } else {
            // General rects
            let build_verts_inds =
                |x: &f32,
                 y: &f32,
                 x2: &f32,
                 y2: &f32,
                 fill: &ColorOrGradient,
                 stroke: &ColorOrGradient,
                 stroke_width: &f32,
                 corner_radius: &f32|
                 -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                    // Create rect path
                    let mut path_builder = lyon::path::Path::builder();
                    let x0 = f32::min(*x, *x2) + origin[0];
                    let x1 = f32::max(*x, *x2) + origin[0];
                    let y0 = f32::min(*y, *y2) + origin[1];
                    let y1 = f32::max(*y, *y2) + origin[1];

                    if *corner_radius > 0.0 {
                        path_builder.add_rounded_rectangle(
                            &Box2D::new(Point2D::new(x0, y0), Point2D::new(x1, y1)),
                            &BorderRadii {
                                top_left: *corner_radius,
                                top_right: *corner_radius,
                                bottom_left: *corner_radius,
                                bottom_right: *corner_radius,
                            },
                            Winding::Positive,
                        );
                    } else {
                        path_builder.add_rectangle(
                            &Box2D::new(Point2D::new(x0, y0), Point2D::new(x1, y1)),
                            Winding::Positive,
                        );
                    }

                    // Apply transform to path
                    let path = path_builder.build();
                    let bbox = bounding_box(&path);

                    // Create vertex/index buffer builder
                    let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
                    let mut builder = BuffersBuilder::new(
                        &mut buffers,
                        VertexPositions {
                            fill: to_color_or_gradient_coord(fill, &grad_coords),
                            stroke: to_color_or_gradient_coord(stroke, &grad_coords),
                            top_left: bbox.min.to_array(),
                            bottom_right: bbox.max.to_array(),
                        },
                    );

                    // Tesselate fill
                    let mut fill_tessellator = FillTessellator::new();
                    let fill_options = FillOptions::default().with_tolerance(0.05);

                    fill_tessellator.tessellate_path(&path, &fill_options, &mut builder)?;

                    // Tesselate stroke
                    if *stroke_width > 0.0 {
                        let mut stroke_tessellator = StrokeTessellator::new();
                        let stroke_options = StrokeOptions::default()
                            .with_tolerance(0.05)
                            .with_line_width(*stroke_width);
                        stroke_tessellator.tessellate_path(&path, &stroke_options, &mut builder)?;
                    }

                    Ok((buffers.vertices, buffers.indices))
                };

            cfg_if::cfg_if! {
                if #[cfg(feature = "rayon")] {
                    par_izip!(
                        mark.x_vec(),
                        mark.y_vec(),
                        mark.x2_vec(),
                        mark.y2_vec(),
                        mark.fill_vec(),
                        mark.stroke_vec(),
                        mark.stroke_width_vec(),
                        mark.corner_radius_vec(),
                    ).map(|(x, y, x2, y2, fill, stroke, stroke_width, corner_radius)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                        build_verts_inds(x, &y, &x2, &y2, &fill, &stroke, &stroke_width, &corner_radius)
                    }).collect::<Result<Vec<_>, AvengerWgpuError>>()?
                } else {
                    izip!(
                        mark.x_iter(),
                        mark.y_iter(),
                        mark.x2_vec(),
                        mark.y2_vec(),
                        mark.fill_iter(),
                        mark.stroke_iter(),
                        mark.stroke_width_iter(),
                        mark.corner_radius_iter(),
                    ).map(|(x, y, x2, y2, fill, stroke, stroke_width, corner_radius)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                            build_verts_inds(x, y, &x2, &y2, fill, stroke, stroke_width, corner_radius)
                        }).collect::<Result<Vec<_>, AvengerWgpuError>>()?
                }
            }
        };

        let start_ind = self.num_indices();
        let inds_len: usize = verts_inds.iter().map(|(_, i)| i.len()).sum();
        let indices_range = (start_ind as u32)..((start_ind + inds_len) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
            image_smooth: true,
            gradient_atlas_index,
            text_atlas_index: None,
        };

        self.verts_inds.extend(verts_inds);
        self.batches.push(batch);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub fn add_path_mark(
        &mut self,
        mark: &ScenePathMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);

        let build_verts_inds = |path: &lyon::path::Path,
                                fill: &ColorOrGradient,
                                stroke: &ColorOrGradient|
         -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
            let bbox = bounding_box(path);

            // Create vertex/index buffer builder
            let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
            let mut builder = BuffersBuilder::new(
                &mut buffers,
                crate::marks::multi::VertexPositions {
                    fill: to_color_or_gradient_coord(fill, &grad_coords),
                    stroke: to_color_or_gradient_coord(stroke, &grad_coords),
                    top_left: bbox.min.to_array(),
                    bottom_right: bbox.max.to_array(),
                },
            );

            // Tesselate fill
            let mut fill_tessellator = FillTessellator::new();
            let fill_options = FillOptions::default().with_tolerance(0.05);

            fill_tessellator.tessellate_path(path, &fill_options, &mut builder)?;

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
                stroke_tessellator.tessellate_path(path, &stroke_options, &mut builder)?;
            }

            Ok((buffers.vertices, buffers.indices))
        };

        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                let verts_inds = par_izip!(
                    mark.transformed_path_iter(origin).collect::<Vec<_>>(),
                    mark.fill_vec(),
                    mark.stroke_vec(),
                ).map(|(path, fill, stroke)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                    build_verts_inds(path, &fill, &stroke)
                }).collect::<Result<Vec<_>, AvengerWgpuError>>()?;
            } else {
                let verts_inds = izip!(
                    mark.transformed_path_iter(origin),
                    mark.fill_iter(),
                    mark.stroke_iter(),
                ).map(|(path, fill, stroke)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                    build_verts_inds(&path, &fill, &stroke)
                }).collect::<Result<Vec<_>, AvengerWgpuError>>()?;
            }
        }

        let start_ind = self.num_indices();
        let inds_len: usize = verts_inds.iter().map(|(_, i)| i.len()).sum();
        let indices_range = (start_ind as u32)..((start_ind + inds_len) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
            image_smooth: true,
            gradient_atlas_index,
            text_atlas_index: None,
        };

        self.verts_inds.extend(verts_inds);
        self.batches.push(batch);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub fn add_symbol_mark(
        &mut self,
        mark: &SceneSymbolMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        let paths = mark.shapes.iter().map(|s| s.as_path()).collect::<Vec<_>>();

        // Compute cradients
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);

        // Find max size
        let max_scale = mark.max_size().sqrt();

        // Compute stroke_width
        let stroke_width = mark.stroke_width.unwrap_or(0.0);

        // Tesselate paths
        let mut shape_verts_inds: Vec<(Vec<SymbolVertex>, Vec<u32>)> = Vec::new();
        for path in paths {
            // Scale path to max size
            let path = path
                .as_ref()
                .clone()
                .transformed(&PathTransform::scale(max_scale, max_scale));

            // Create vertex/index buffer builder
            let mut buffers: VertexBuffers<SymbolVertex, u32> = VertexBuffers::new();
            let mut builder =
                BuffersBuilder::new(&mut buffers, SymbolVertexPositions { scale: max_scale });

            // Tesselate fill
            let mut fill_tessellator = FillTessellator::new();
            let fill_options = FillOptions::default().with_tolerance(0.1);

            fill_tessellator.tessellate_path(&path, &fill_options, &mut builder)?;

            // Tesselate stroke
            if stroke_width > 0.0 {
                let mut stroke_tessellator = StrokeTessellator::new();
                let stroke_options = StrokeOptions::default()
                    .with_tolerance(0.1)
                    .with_line_join(LineJoin::Miter)
                    .with_line_cap(LineCap::Butt)
                    .with_line_width(NORMALIZED_SYMBOL_STROKE_WIDTH);
                stroke_tessellator.tessellate_path(&path, &stroke_options, &mut builder)?;
            }

            shape_verts_inds.push((buffers.vertices, buffers.indices));
        }

        // Builder function that we'll call from either single-threaded or parallel iterations paths
        let build_verts_inds = |x: &f32,
                                y: &f32,
                                fill: &ColorOrGradient,
                                size: &f32,
                                stroke: &ColorOrGradient,
                                angle: &f32,
                                shape_index: &usize|
         -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
            let (symbol_verts, indices) = &shape_verts_inds[*shape_index];
            let fill = to_color_or_gradient_coord(fill, &grad_coords);
            let stroke = to_color_or_gradient_coord(stroke, &grad_coords);

            let multi_verts = symbol_verts
                .iter()
                .map(|sv| {
                    sv.as_multi_vertex(
                        *size,
                        *x + origin[0],
                        *y + origin[1],
                        *angle,
                        fill,
                        stroke,
                        stroke_width,
                    )
                })
                .collect::<Vec<_>>();
            Ok((multi_verts, indices.clone()))
        };

        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                let verts_inds = par_izip!(
                    mark.x_vec(),
                    mark.y_vec(),
                    mark.fill_vec(),
                    mark.size_vec(),
                    mark.stroke_vec(),
                    mark.angle_vec(),
                    mark.shape_index_vec(),
                ).map(|(x, y, fill, size, stroke, angle, shape_index)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                    build_verts_inds(x, &y, &fill, &size, &stroke, &angle, &shape_index)
                }).collect::<Result<Vec<_>, AvengerWgpuError>>()?;
            } else {
                let verts_inds = izip!(
                    mark.x_iter(),
                    mark.y_iter(),
                    mark.fill_iter(),
                    mark.size_iter(),
                    mark.stroke_iter(),
                    mark.angle_iter(),
                    mark.shape_index_iter(),
                ).map(|(x, y, fill, size, stroke, angle, shape_index)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                    build_verts_inds(&x, &y, fill, size, stroke, angle, shape_index)
                }).collect::<Result<Vec<_>, AvengerWgpuError>>()?;
            }
        };

        let start_ind = self.num_indices();
        let inds_len: usize = verts_inds.iter().map(|(_, i)| i.len()).sum();
        let indices_range = (start_ind as u32)..((start_ind + inds_len) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
            image_smooth: true,
            gradient_atlas_index,
            text_atlas_index: None,
        };

        self.verts_inds.extend(verts_inds);
        self.batches.push(batch);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub fn add_line_mark(
        &mut self,
        mark: &SceneLineMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);

        let path = mark.transformed_path(origin);

        let mut verts: Vec<MultiVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        let bbox = bounding_box(&path);
        // Create vertex/index buffer builder
        let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
        let mut buffers_builder = BuffersBuilder::new(
            &mut buffers,
            VertexPositions {
                fill: [0.0, 0.0, 0.0, 0.0],
                stroke: to_color_or_gradient_coord(&mark.stroke, &grad_coords),
                top_left: bbox.min.to_array(),
                bottom_right: bbox.max.to_array(),
            },
        );

        // Tesselate path
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
            .with_line_width(mark.stroke_width);
        stroke_tessellator.tessellate_path(&path, &stroke_options, &mut buffers_builder)?;

        let index_offset = verts.len() as u32;
        verts.extend(buffers.vertices);
        indices.extend(buffers.indices.into_iter().map(|i| i + index_offset));

        let start_ind = self.num_indices();
        let indices_range = (start_ind as u32)..((start_ind + indices.len()) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
            image_smooth: true,
            gradient_atlas_index,
            text_atlas_index: None,
        };

        self.verts_inds.push((verts, indices));
        self.batches.push(batch);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub fn add_area_mark(
        &mut self,
        mark: &SceneAreaMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);

        let fill_path = mark.transformed_path(origin);
        let stroke_path = mark.transformed_stroke_path(origin);

        let bbox = bounding_box(&fill_path);

        // Create vertex/index buffer builder
        let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
        let mut buffers_builder = BuffersBuilder::new(
            &mut buffers,
            VertexPositions {
                fill: to_color_or_gradient_coord(&mark.fill, &grad_coords),
                stroke: to_color_or_gradient_coord(&mark.stroke, &grad_coords),
                top_left: bbox.min.to_array(),
                bottom_right: bbox.max.to_array(),
            },
        );

        // Tessellate fill
        let mut fill_tessellator = FillTessellator::new();
        let fill_options = FillOptions::default().with_tolerance(0.05);
        fill_tessellator.tessellate_path(&fill_path, &fill_options, &mut buffers_builder)?;

        // Tessellate path
        if mark.stroke_width > 0.0 {
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
                .with_line_width(mark.stroke_width);
            stroke_tessellator.tessellate_path(
                &stroke_path,
                &stroke_options,
                &mut buffers_builder,
            )?;
        }

        let start_ind = self.num_indices();
        let indices_range = (start_ind as u32)..((start_ind + buffers.indices.len()) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
            image_smooth: true,
            gradient_atlas_index,
            text_atlas_index: None,
        };

        self.verts_inds.push((buffers.vertices, buffers.indices));
        self.batches.push(batch);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub fn add_trail_mark(
        &mut self,
        mark: &SceneTrailMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);

        let path = mark.transformed_path(origin);
        let bbox = bounding_box(&path);

        // Create vertex/index buffer builder
        let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
        let mut buffers_builder = BuffersBuilder::new(
            &mut buffers,
            VertexPositions {
                fill: [0.0, 0.0, 0.0, 0.0],
                stroke: to_color_or_gradient_coord(&mark.stroke, &grad_coords),
                top_left: bbox.min.to_array(),
                bottom_right: bbox.max.to_array(),
            },
        );

        // Tesselate path
        let mut stroke_tessellator = StrokeTessellator::new();
        let stroke_options = StrokeOptions::default()
            .with_tolerance(0.05)
            .with_line_join(LineJoin::Round)
            .with_line_cap(LineCap::Round)
            .with_variable_line_width(0);
        stroke_tessellator.tessellate_path(&path, &stroke_options, &mut buffers_builder)?;

        // Variable-width stroke joins can fold back on themselves. Keep the
        // front-facing triangles that the original culled pipeline rendered;
        // drawing the reversed folds applies translucent paint twice.
        buffers.indices = buffers
            .indices
            .chunks_exact(3)
            .filter(|triangle| {
                let a = buffers.vertices[triangle[0] as usize].position;
                let b = buffers.vertices[triangle[1] as usize].position;
                let c = buffers.vertices[triangle[2] as usize].position;
                (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) <= 0.0
            })
            .flatten()
            .copied()
            .collect();

        let start_ind = self.num_indices();
        let indices_range = (start_ind as u32)..((start_ind + buffers.indices.len()) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
            image_smooth: true,
            gradient_atlas_index,
            text_atlas_index: None,
        };

        self.verts_inds.push((buffers.vertices, buffers.indices));
        self.batches.push(batch);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub fn add_arc_mark(
        &mut self,
        mark: &SceneArcMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);

        let verts_inds = izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter(),
        )
        .map(
            |(
                path,
                fill,
                stroke,
                stroke_width,
            )|
             -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                // Compute bounding box
                let bbox = bounding_box(&path);

                // Create vertex/index buffer builder
                let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
                let mut builder = BuffersBuilder::new(
                    &mut buffers,
                    VertexPositions {
                        fill: to_color_or_gradient_coord(fill, &grad_coords),
                        stroke: to_color_or_gradient_coord(stroke, &grad_coords),
                        top_left: bbox.min.to_array(),
                        bottom_right: bbox.max.to_array(),
                    },
                );

                // Tesselate fill
                let mut fill_tessellator = FillTessellator::new();
                let fill_options = FillOptions::default().with_tolerance(0.05);
                fill_tessellator.tessellate_path(&path, &fill_options, &mut builder)?;

                // Tesselate stroke
                if *stroke_width > 0.0 {
                    let mut stroke_tessellator = StrokeTessellator::new();
                    let stroke_options = StrokeOptions::default()
                        .with_tolerance(0.05)
                        .with_line_join(LineJoin::Miter)
                        .with_line_cap(LineCap::Butt)
                        .with_line_width(*stroke_width);
                    stroke_tessellator.tessellate_path(&path, &stroke_options, &mut builder)?;
                }

                Ok((buffers.vertices, buffers.indices))
            },
        )
        .collect::<Result<Vec<_>, AvengerWgpuError>>()?;

        let start_ind = self.num_indices();
        let inds_len: usize = verts_inds.iter().map(|(_, i)| i.len()).sum();
        let indices_range = (start_ind as u32)..((start_ind + inds_len) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
            image_smooth: true,
            gradient_atlas_index,
            text_atlas_index: None,
        };

        self.verts_inds.extend(verts_inds);
        self.batches.push(batch);
        Ok(())
    }

    /// Build mark batches from glyph registrations produced by the shared text atlas.
    ///
    /// The glyph-registration step (rasterizing + allocating atlas slots) lives on the
    /// `Canvas` so that a single text atlas is shared across all multi-renderers. This
    /// method consumes the resulting per-page [`TextAtlasRegistration`]s and turns them
    /// into [`MultiMarkBatch`]es, exactly as the old `add_text_mark` did.
    ///
    /// `clip` is the inherited group clip and `mark_clip` is the text mark's own
    /// `clip` flag; both are needed to reproduce `clip.maybe_clip(mark.clip)` and
    /// `add_clip_path(clip, mark.clip)`.
    #[tracing::instrument(skip_all)]
    pub fn add_text_registrations(
        &mut self,
        registrations: Vec<crate::marks::text::TextAtlasRegistration>,
        clip: &Clip,
        mark_clip: bool,
    ) -> Result<(), AvengerWgpuError> {
        // Construct batches, one batch per text atlas index
        let start_ind = self.num_indices() as u32;
        let mut next_batch = MultiMarkBatch {
            indices_range: start_ind..start_ind,
            clip: clip.maybe_clip(mark_clip),
            clip_indices_range: self.add_clip_path(clip, mark_clip)?,
            image_atlas_index: None,
            image_smooth: true,
            gradient_atlas_index: None,
            text_atlas_index: None,
        };

        for registration in registrations {
            let atlas_index = registration.atlas_index;
            let verts = registration.verts;
            let inds = registration.indices;

            // (atlas_index, verts, inds)
            if next_batch.text_atlas_index.unwrap_or(atlas_index) == atlas_index {
                // update next batch with atlas index and inds range
                next_batch.text_atlas_index = Some(atlas_index);
                next_batch.indices_range = next_batch.indices_range.start
                    ..(next_batch.indices_range.end + inds.len() as u32);
            } else {
                // create new batch
                let start_ind = next_batch.indices_range.end;
                // Initialize new next_batch and swap to avoid extra mem copy
                let mut full_batch = MultiMarkBatch {
                    indices_range: start_ind..(start_ind + inds.len() as u32),
                    clip: clip.maybe_clip(mark_clip),
                    clip_indices_range: self.add_clip_path(clip, mark_clip)?,
                    image_atlas_index: None,
                    image_smooth: true,
                    gradient_atlas_index: None,
                    text_atlas_index: Some(atlas_index),
                };
                std::mem::swap(&mut full_batch, &mut next_batch);
                self.batches.push(full_batch);
            }

            // Add verts and indices
            self.verts_inds.push((verts, inds))
        }

        self.batches.push(next_batch);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub(crate) fn add_text_leaders(
        &mut self,
        leaders: Vec<TextLeaderRenderItem>,
        clip: &Clip,
        mark_clip: bool,
    ) -> Result<(), AvengerWgpuError> {
        if leaders.is_empty() {
            return Ok(());
        }

        let verts_inds = leaders
            .iter()
            .map(tessellate_text_leader)
            .collect::<Result<Vec<_>, AvengerWgpuError>>()?;

        let start_ind = self.num_indices();
        let inds_len: usize = verts_inds.iter().map(|(_, indices)| indices.len()).sum();
        if inds_len == 0 {
            return Ok(());
        }
        let indices_range = (start_ind as u32)..((start_ind + inds_len) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark_clip),
            clip_indices_range: self.add_clip_path(clip, mark_clip)?,
            image_atlas_index: None,
            image_smooth: true,
            gradient_atlas_index: None,
            text_atlas_index: None,
        };

        self.verts_inds.extend(verts_inds);
        self.batches.push(batch);
        Ok(())
    }

    fn num_indices(&self) -> usize {
        self.verts_inds.iter().map(|(_, inds)| inds.len()).sum()
    }

    fn num_clip_indices(&self) -> usize {
        self.clip_verts_inds
            .iter()
            .map(|(_, inds)| inds.len())
            .sum()
    }

    #[tracing::instrument(skip_all)]
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        device: &Device,
        queue: &Queue,
        texture_format: TextureFormat,
        sample_count: u32,
        render_target_extent: Extent3d,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
        text_bind_groups: &[BindGroup],
    ) -> Result<CommandBuffer, AvengerWgpuError> {
        let resources = MultiMarkRenderResources::new(device, texture_format, sample_count);
        self.render_with_resources(
            device,
            queue,
            render_target_extent,
            texture_view,
            resolve_target,
            &resources,
            text_bind_groups,
        )
    }

    /// Build the per-frame GPU resources (uniform, gradient/image atlas bind groups,
    /// stencil buffer, and the flattened vertex/index/clip buffers) once, so a single
    /// shared renderer's geometry/atlas/uniform are uploaded a single time per frame.
    pub(crate) fn prepare(
        &self,
        device: &Device,
        queue: &Queue,
        render_target_extent: Extent3d,
        resources: &MultiMarkRenderResources,
    ) -> Result<PreparedMulti, AvengerWgpuError> {
        // Uniforms
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Multi Uniform Buffer"),
            contents: bytemuck::cast_slice(&[self.uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &resources.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
            label: Some("uniform_bind_group"),
        });

        // Gradient Textures
        let (grad_texture_size, grad_images) = self.gradient_atlas_builder.build();
        let gradient_texture_bind_groups = Self::make_texture_bind_groups(
            device,
            queue,
            &resources.texture_layout,
            grad_texture_size,
            &grad_images,
            wgpu::FilterMode::Nearest,
            wgpu::FilterMode::Nearest,
        );

        // Image Textures (one texture + upload per page, two sampler
        // variants).
        let (image_texture_size, image_images) = self.image_atlas_builder.build();
        let (image_texture_bind_groups, image_texture_bind_groups_nearest) =
            Self::make_dual_sampler_texture_bind_groups(
                device,
                queue,
                &resources.texture_layout,
                image_texture_size,
                &image_images,
            );

        // Path clips need a stencil buffer.
        let uses_stencil = self.num_clip_indices() > 0;
        let stencil_buffer = uses_stencil.then(|| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Stencil buffer"),
                size: Extent3d {
                    width: render_target_extent.width,
                    height: render_target_extent.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: resources.sample_count(),
                dimension: wgpu::TextureDimension::D2,
                format: STENCIL_ATTACHMENT_FORMAT,
                view_formats: &[],
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            })
        });

        // Flatten verts and inds
        let num_verts: usize = self.verts_inds.iter().map(|(v, _)| v.len()).sum();
        let num_inds: usize = self.verts_inds.iter().map(|(_, inds)| inds.len()).sum();
        let mut verticies: Vec<MultiVertex> = Vec::with_capacity(num_verts);
        let mut indices: Vec<u32> = Vec::with_capacity(num_inds);
        for (vs, inds) in &self.verts_inds {
            let offset = verticies.len() as u32;
            indices.extend(inds.iter().map(|i| *i + offset));
            verticies.extend(vs);
        }

        let num_clip_verts = self.clip_verts_inds.iter().map(|(v, _)| v.len()).sum();
        let num_clip_inds = self
            .clip_verts_inds
            .iter()
            .map(|(_, inds)| inds.len())
            .sum();
        let mut clip_verticies: Vec<MultiVertex> = Vec::with_capacity(num_clip_verts);
        let mut clip_indices: Vec<u32> = Vec::with_capacity(num_clip_inds);
        for (vs, inds) in &self.clip_verts_inds {
            let offset = clip_verticies.len() as u32;
            clip_indices.extend(inds.iter().map(|i| *i + offset));
            clip_verticies.extend(vs);
        }

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
        let clip_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Clip Vertex Buffer"),
            contents: bytemuck::cast_slice(clip_verticies.as_slice()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let clip_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Clip Index Buffer"),
            contents: bytemuck::cast_slice(clip_indices.as_slice()),
            usage: wgpu::BufferUsages::INDEX,
        });

        let prepared = PreparedMulti {
            uniform_bind_group,
            gradient_texture_bind_groups,
            image_texture_bind_groups,
            image_texture_bind_groups_nearest,
            stencil_buffer,
            vertex_buffer,
            index_buffer,
            clip_vertex_buffer,
            clip_index_buffer,
        };

        Ok(prepared)
    }

    fn image_texture_bind_group<'a>(
        prepared: &'a PreparedMulti,
        batch: &MultiMarkBatch,
    ) -> &'a BindGroup {
        let index = batch.image_atlas_index.unwrap_or(0);
        if batch.image_smooth {
            &prepared.image_texture_bind_groups[index]
        } else {
            &prepared.image_texture_bind_groups_nearest[index]
        }
    }

    /// Encode the draws for a set of batch ranges into a caller-provided command
    /// encoder. See `encode_multi_ranges` for the batching and stencil semantics.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_multi_ranges_into(
        &self,
        mark_encoder: &mut wgpu::CommandEncoder,
        render_target_extent: Extent3d,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
        resources: &MultiMarkRenderResources,
        text_bind_groups: &[BindGroup],
        prepared: &PreparedMulti,
        ranges: &[std::ops::Range<usize>],
    ) {
        // Batch indices in draw order across all ranges.
        let mut order: Vec<usize> = Vec::new();
        for r in ranges {
            order.extend(r.clone());
        }
        if order.is_empty() || prepared.vertex_buffer.size() == 0 {
            return;
        }

        let depth_view = prepared
            .stencil_buffer
            .as_ref()
            .map(|buffer| buffer.create_view(&Default::default()));
        let scale = self.uniform.scale;
        let cw = render_target_extent.width;
        let ch = render_target_extent.height;

        let mut i = 0usize;
        while i < order.len() {
            let bi = order[i];
            if self.batches[bi].clip_indices_range.is_some() {
                // Dedicated pass for a Path/stencil clip (fresh Clear(0) stencil).
                let dsa = depth_view
                    .as_ref()
                    .map(|view| wgpu::RenderPassDepthStencilAttachment {
                        view,
                        depth_ops: if cfg!(feature = "deno") {
                            Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(0.0),
                                store: wgpu::StoreOp::Discard,
                            })
                        } else {
                            None
                        },
                        stencil_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0),
                            store: wgpu::StoreOp::Store,
                        }),
                    });
                let batch = &self.batches[bi];
                let mut rp = mark_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Multi Mark Render Pass (stencil)"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: texture_view,
                        resolve_target,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: dsa,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                rp.set_bind_group(0, &prepared.uniform_bind_group, &[]);
                rp.set_bind_group(
                    1,
                    &prepared.gradient_texture_bind_groups[batch.gradient_atlas_index.unwrap_or(0)],
                    &[],
                );
                rp.set_bind_group(2, Self::image_texture_bind_group(prepared, batch), &[]);
                rp.set_bind_group(
                    3,
                    &text_bind_groups[batch.text_atlas_index.unwrap_or(0)],
                    &[],
                );
                // Draw the clip shape into the stencil (ref 1), then the mark.
                rp.set_stencil_reference(1);
                rp.set_pipeline(&resources.stencil_pipeline);
                rp.set_vertex_buffer(0, prepared.clip_vertex_buffer.slice(..));
                rp.set_index_buffer(
                    prepared.clip_index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                rp.draw_indexed(batch.clip_indices_range.clone().unwrap(), 0, 0..1);
                rp.set_pipeline(&resources.stencil_render_pipeline);
                rp.set_vertex_buffer(0, prepared.vertex_buffer.slice(..));
                rp.set_index_buffer(prepared.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                Self::apply_scissor(&mut rp, &batch.clip, scale, cw, ch);
                rp.draw_indexed(batch.indices_range.clone(), 0, 0..1);
                i += 1;
            } else {
                // One pass for a run of non-stencil batches.
                let mut rp = mark_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Multi Mark Render Pass"),
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
                rp.set_pipeline(&resources.render_pipeline);
                rp.set_bind_group(0, &prepared.uniform_bind_group, &[]);
                rp.set_vertex_buffer(0, prepared.vertex_buffer.slice(..));
                rp.set_index_buffer(prepared.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                while i < order.len() {
                    let bi = order[i];
                    let batch = &self.batches[bi];
                    if batch.clip_indices_range.is_some() {
                        break;
                    }
                    // Tile batches swap in the texture-array pipeline
                    // mid-pass; groups 0/1/3 share layouts with the main
                    // pipeline so their bindings stay valid.

                    rp.set_bind_group(
                        1,
                        &prepared.gradient_texture_bind_groups
                            [batch.gradient_atlas_index.unwrap_or(0)],
                        &[],
                    );
                    {
                        rp.set_bind_group(2, Self::image_texture_bind_group(prepared, batch), &[]);
                    }
                    rp.set_bind_group(
                        3,
                        &text_bind_groups[batch.text_atlas_index.unwrap_or(0)],
                        &[],
                    );
                    Self::apply_scissor(&mut rp, &batch.clip, scale, cw, ch);
                    rp.draw_indexed(batch.indices_range.clone(), 0, 0..1);
                    i += 1;
                }
            }
        }
    }

    /// Set the scissor rect for a batch's clip, resetting to the full target for
    /// non-rect clips (so a shared render pass matches a fresh per-batch pass).
    fn apply_scissor(rp: &mut wgpu::RenderPass<'_>, clip: &Clip, scale: f32, cw: u32, ch: u32) {
        if let Clip::Rect {
            x,
            y,
            width,
            height,
        } = clip
        {
            let px = (*x * scale) as u32;
            let py = (*y * scale) as u32;
            let pw = (*width * scale) as u32;
            let ph = (*height * scale) as u32;
            let cx = px.min(cw);
            let cy = py.min(ch);
            rp.set_scissor_rect(cx, cy, pw.min(cw - cx), ph.min(ch - cy));
        } else {
            rp.set_scissor_rect(0, 0, cw, ch);
        }
    }

    #[tracing::instrument(skip_all)]
    #[allow(clippy::too_many_arguments)]
    pub fn render_with_resources(
        &self,
        device: &Device,
        queue: &Queue,
        render_target_extent: Extent3d,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
        resources: &MultiMarkRenderResources,
        text_bind_groups: &[BindGroup],
    ) -> Result<CommandBuffer, AvengerWgpuError> {
        let timing_enabled =
            tracing::enabled!(target: "avenger_wgpu::render_breakdown", tracing::Level::DEBUG);
        let total_start = timing_enabled.then(Instant::now);
        let mut checkpoint = total_start;

        // Uniforms
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Multi Uniform Buffer"),
            contents: bytemuck::cast_slice(&[self.uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &resources.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
            label: Some("uniform_bind_group"),
        });
        let uniform_setup_us = checkpoint_us(&mut checkpoint);

        // Gradient Textures
        let (grad_texture_size, grad_images) = self.gradient_atlas_builder.build();
        let gradient_texture_bind_groups = Self::make_texture_bind_groups(
            device,
            queue,
            &resources.texture_layout,
            grad_texture_size,
            &grad_images,
            wgpu::FilterMode::Nearest,
            wgpu::FilterMode::Nearest,
        );
        let gradient_setup_us = checkpoint_us(&mut checkpoint);

        // Image Textures (one texture + upload per page, two sampler
        // variants).
        let (image_texture_size, image_images) = self.image_atlas_builder.build();
        let (image_texture_bind_groups, image_texture_bind_groups_nearest) =
            Self::make_dual_sampler_texture_bind_groups(
                device,
                queue,
                &resources.texture_layout,
                image_texture_size,
                &image_images,
            );
        let image_setup_us = checkpoint_us(&mut checkpoint);

        // Text textures are built once per frame and shared across all multi-renderers;
        // the canvas builds them and passes the bind groups in via `text_bind_groups`.
        let text_setup_us = checkpoint_us(&mut checkpoint);

        let uses_stencil = self.num_clip_indices() > 0;
        let stencil_buffer = uses_stencil.then(|| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Stencil buffer"),
                size: Extent3d {
                    width: render_target_extent.width,
                    height: render_target_extent.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: resources.sample_count(),
                dimension: wgpu::TextureDimension::D2,
                format: STENCIL_ATTACHMENT_FORMAT,
                view_formats: &[],
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            })
        });
        let pipeline_setup_us = checkpoint_us(&mut checkpoint);

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

        let num_clip_verts = self.clip_verts_inds.iter().map(|(v, _)| v.len()).sum();
        let num_clip_inds = self
            .clip_verts_inds
            .iter()
            .map(|(_, inds)| inds.len())
            .sum();
        let mut clip_verticies: Vec<MultiVertex> = Vec::with_capacity(num_clip_verts);
        let mut clip_indices: Vec<u32> = Vec::with_capacity(num_clip_inds);

        for (vs, inds) in &self.clip_verts_inds {
            let offset = clip_verticies.len() as u32;
            clip_indices.extend(inds.iter().map(|i| *i + offset));
            clip_verticies.extend(vs);
        }
        let flatten_us = checkpoint_us(&mut checkpoint);

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

        let clip_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Clip Vertex Buffer"),
            contents: bytemuck::cast_slice(clip_verticies.as_slice()),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let clip_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Clip Index Buffer"),
            contents: bytemuck::cast_slice(clip_indices.as_slice()),
            usage: wgpu::BufferUsages::INDEX,
        });
        let buffer_upload_us = checkpoint_us(&mut checkpoint);

        let prepared = PreparedMulti {
            uniform_bind_group,
            gradient_texture_bind_groups,
            image_texture_bind_groups,
            image_texture_bind_groups_nearest,
            stencil_buffer,
            vertex_buffer,
            index_buffer,
            clip_vertex_buffer,
            clip_index_buffer,
        };

        // Create command encoder for marks
        let mut mark_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Multi Mark Render Encoder"),
        });
        self.encode_multi_ranges_into(
            &mut mark_encoder,
            render_target_extent,
            texture_view,
            resolve_target,
            resources,
            text_bind_groups,
            &prepared,
            std::slice::from_ref(&(0..self.batches.len())),
        );
        let encode_us = checkpoint_us(&mut checkpoint);

        if let Some(start) = total_start {
            tracing::debug!(
                target: "avenger_wgpu::render_breakdown",
                renderer = "multi",
                total_ms = start.elapsed().as_secs_f64() * 1000.0,
                uniform_ms = us_to_ms(uniform_setup_us),
                gradient_ms = us_to_ms(gradient_setup_us),
                image_ms = us_to_ms(image_setup_us),
                text_ms = us_to_ms(text_setup_us),
                pipeline_ms = us_to_ms(pipeline_setup_us),
                flatten_ms = us_to_ms(flatten_us),
                buffer_upload_ms = us_to_ms(buffer_upload_us),
                encode_ms = us_to_ms(encode_us),
                vertex_chunks = self.verts_inds.len(),
                clip_chunks = self.clip_verts_inds.len(),
                batch_count = self.batches.len(),
                vertex_count = num_verts,
                index_count = num_inds,
                clip_vertex_count = num_clip_verts,
                clip_index_count = num_clip_inds,
                gradient_atlas_count = grad_images.len(),
                image_atlas_count = image_images.len(),
                text_atlas_count = text_bind_groups.len(),
                uses_stencil,
                "wgpu.render.renderer"
            );
        }

        Ok(mark_encoder.finish())
    }

    pub(crate) fn make_text_bind_groups_dual_sampler(
        device: &Device,
        queue: &Queue,
        texture_bind_group_layout: &BindGroupLayout,
        size: Extent3d,
        images: &[DynamicImage],
    ) -> Vec<BindGroup> {
        // Create texture for each image
        let mut texture_bind_groups: Vec<BindGroup> = Vec::new();

        for image in images {
            // Create Texture
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                label: Some("text_texture"),
                view_formats: &[],
            });
            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Create linear sampler
            let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            });

            // Create nearest sampler
            let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            });

            let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&linear_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&nearest_sampler),
                    },
                ],
                label: Some("text_dual_sampler_bind_group"),
            });

            queue.write_texture(
                // Tells wgpu where to copy the pixel data
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                // The actual pixel data
                image.to_rgba8().as_raw(),
                // The layout of the texture
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * image.width()),
                    rows_per_image: Some(image.height()),
                },
                size,
            );

            texture_bind_groups.push(texture_bind_group);
        }

        texture_bind_groups
    }

    /// One texture + upload per atlas page, shared by a linear-sampler
    /// and a nearest-sampler bind group (samplers don't need separate
    /// textures, so both filter variants read the same upload).
    fn make_dual_sampler_texture_bind_groups(
        device: &Device,
        queue: &Queue,
        texture_bind_group_layout: &BindGroupLayout,
        size: Extent3d,
        images: &[DynamicImage],
    ) -> (Vec<BindGroup>, Vec<BindGroup>) {
        let mut linear_bind_groups: Vec<BindGroup> = Vec::with_capacity(images.len());
        let mut nearest_bind_groups: Vec<BindGroup> = Vec::with_capacity(images.len());
        let sampler_for = |filter: wgpu::FilterMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            })
        };
        let linear_sampler = sampler_for(wgpu::FilterMode::Linear);
        let nearest_sampler = sampler_for(wgpu::FilterMode::Nearest);

        for image in images {
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
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                image.to_rgba8().as_raw(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * image.width()),
                    rows_per_image: Some(image.height()),
                },
                size,
            );
            for (sampler, bind_groups) in [
                (&linear_sampler, &mut linear_bind_groups),
                (&nearest_sampler, &mut nearest_bind_groups),
            ] {
                bind_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    layout: texture_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&texture_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                    ],
                    label: Some("texture_bind_group"),
                }));
            }
        }
        (linear_bind_groups, nearest_bind_groups)
    }

    fn make_texture_bind_groups(
        device: &Device,
        queue: &Queue,
        texture_bind_group_layout: &BindGroupLayout,
        size: Extent3d,
        images: &[DynamicImage],
        mag_filter: wgpu::FilterMode,
        min_filter: wgpu::FilterMode,
    ) -> Vec<BindGroup> {
        // Create texture for each image
        let mut texture_bind_groups: Vec<BindGroup> = Vec::new();

        for image in images {
            // Create Texture
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
                mag_filter,
                min_filter,
                mipmap_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            });

            let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: texture_bind_group_layout,
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
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                // The actual pixel data
                image.to_rgba8().as_raw(),
                // The layout of the texture
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * image.width()),
                    rows_per_image: Some(image.height()),
                },
                size,
            );

            texture_bind_groups.push(texture_bind_group);
        }

        texture_bind_groups
    }
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

fn tessellate_text_leader(
    leader: &TextLeaderRenderItem,
) -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
    let stroke_color = match &leader.stroke {
        ColorOrGradient::Color(color) => *color,
        ColorOrGradient::GradientIndex(_) => [0.0, 0.0, 0.0, 0.0],
    };
    let spine_path = leader_path_to_lyon(&leader.geometry.spine);
    let spine_path = if let Some(dash) = &leader.stroke_dash {
        dash_paths(std::iter::once(&spine_path), dash)
    } else {
        spine_path
    };

    let bbox = bounding_box(&spine_path);
    let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
    let mut builder = BuffersBuilder::new(
        &mut buffers,
        VertexPositions {
            fill: stroke_color,
            stroke: stroke_color,
            top_left: bbox.min.to_array(),
            bottom_right: bbox.max.to_array(),
        },
    );

    let mut stroke_tessellator = StrokeTessellator::new();
    let stroke_options = StrokeOptions::default()
        .with_tolerance(0.05)
        .with_line_join(to_line_join(leader.stroke_join))
        .with_line_cap(to_line_cap(leader.stroke_cap))
        .with_line_width(leader.stroke_width.max(0.0));
    stroke_tessellator.tessellate_path(&spine_path, &stroke_options, &mut builder)?;

    if let Some(arrowhead) = &leader.geometry.arrowhead {
        match arrowhead {
            TextLeaderArrowhead::Open { .. } => {
                let arrow_path = arrowhead_to_lyon(arrowhead);
                let arrow_options = StrokeOptions::default()
                    .with_tolerance(0.05)
                    .with_line_join(to_line_join(leader.stroke_join))
                    .with_line_cap(to_line_cap(leader.stroke_cap))
                    .with_line_width(leader.stroke_width.max(0.0));
                stroke_tessellator.tessellate_path(&arrow_path, &arrow_options, &mut builder)?;
            }
            TextLeaderArrowhead::Triangle { .. } => {
                let arrow_path = arrowhead_to_lyon(arrowhead);
                let mut fill_tessellator = FillTessellator::new();
                let fill_options = FillOptions::default().with_tolerance(0.05);
                fill_tessellator.tessellate_path(&arrow_path, &fill_options, &mut builder)?;
            }
        }
    }

    Ok((buffers.vertices, buffers.indices))
}

fn leader_path_to_lyon(path: &TextLeaderPath) -> Path {
    let mut builder = Path::builder();
    match path {
        TextLeaderPath::Line { start, end } => {
            builder.begin(point(start[0], start[1]));
            builder.line_to(point(end[0], end[1]));
            builder.end(false);
        }
        TextLeaderPath::Polyline { points } => {
            if let Some(first) = points.first() {
                builder.begin(point(first[0], first[1]));
                for point_value in points.iter().skip(1) {
                    builder.line_to(point(point_value[0], point_value[1]));
                }
                builder.end(false);
            }
        }
        TextLeaderPath::Cubic {
            start,
            ctrl1,
            ctrl2,
            end,
        } => {
            builder.begin(point(start[0], start[1]));
            builder.cubic_bezier_to(
                point(ctrl1[0], ctrl1[1]),
                point(ctrl2[0], ctrl2[1]),
                point(end[0], end[1]),
            );
            builder.end(false);
        }
    }
    builder.build()
}

fn arrowhead_to_lyon(arrowhead: &TextLeaderArrowhead) -> Path {
    let mut builder = Path::builder();
    match arrowhead {
        TextLeaderArrowhead::Open { left, right } => {
            builder.begin(point(left[0][0], left[0][1]));
            builder.line_to(point(left[1][0], left[1][1]));
            builder.end(false);
            builder.begin(point(right[0][0], right[0][1]));
            builder.line_to(point(right[1][0], right[1][1]));
            builder.end(false);
        }
        TextLeaderArrowhead::Triangle { points } => {
            builder.begin(point(points[0][0], points[0][1]));
            builder.line_to(point(points[1][0], points[1][1]));
            builder.line_to(point(points[2][0], points[2][1]));
            builder.close();
        }
    }
    builder.build()
}

fn to_line_cap(cap: StrokeCap) -> LineCap {
    match cap {
        StrokeCap::Butt => LineCap::Butt,
        StrokeCap::Round => LineCap::Round,
        StrokeCap::Square => LineCap::Square,
    }
}

fn to_line_join(join: StrokeJoin) -> LineJoin {
    match join {
        StrokeJoin::Miter => LineJoin::Miter,
        StrokeJoin::Round => LineJoin::Round,
        StrokeJoin::Bevel => LineJoin::Bevel,
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

// Symbol vertex construction that takes line width into account
pub struct SymbolVertexPositions {
    scale: f32,
}

impl FillVertexConstructor<SymbolVertex> for SymbolVertexPositions {
    fn new_vertex(&mut self, vertex: FillVertex) -> SymbolVertex {
        SymbolVertex {
            position: [vertex.position().x, vertex.position().y].into(),
            normal: None,
            scale: self.scale,
        }
    }
}

impl StrokeVertexConstructor<SymbolVertex> for SymbolVertexPositions {
    fn new_vertex(&mut self, vertex: StrokeVertex) -> SymbolVertex {
        SymbolVertex {
            position: [vertex.position().x, vertex.position().y].into(),
            normal: Some(vertex.normal()),
            scale: self.scale,
        }
    }
}

pub struct SymbolVertex {
    position: Point2D<f32, UnknownUnit>,
    normal: Option<Vector2D<f32, UnknownUnit>>,
    scale: f32,
}

impl SymbolVertex {
    #[allow(clippy::too_many_arguments)]
    pub fn as_multi_vertex(
        &self,
        size: f32,
        x: f32,
        y: f32,
        angle: f32,
        fill: [f32; 4],
        stroke: [f32; 4],
        line_width: f32,
    ) -> MultiVertex {
        let angle = Angle::degrees(angle);
        let absolue_scale = size.sqrt();
        let relative_scale = absolue_scale / self.scale;

        // Scale
        let mut transform = PathTransform::scale(relative_scale, relative_scale);

        // Compute adjustment factor for stroke vertices to compensate for scaling and
        // achieve the correct final line width.
        let color = if let Some(normal) = self.normal {
            let scaled_line_width = relative_scale * NORMALIZED_SYMBOL_STROKE_WIDTH;
            let line_width_adjustment = normal.mul((line_width - scaled_line_width) / 2.0);
            transform = transform.then_translate(line_width_adjustment);
            stroke
        } else {
            fill
        };

        // Rotate then Translate
        transform = transform
            .then_rotate(angle)
            .then_translate(Vector2D::new(x, y));
        let position = transform.transform_point(self.position);

        MultiVertex {
            position: position.to_array(),
            color,
            top_left: [x - absolue_scale / 2.0, y - absolue_scale / 2.0],
            bottom_right: [x + absolue_scale / 2.0, y + absolue_scale / 2.0],
        }
    }
}
