use avenger_common::types::LYON_SCENE_MITER_LIMIT;
use avenger_scenegraph::render::{
    items::path_is_finite,
    paint::GradientBounds,
    stroke::{dash_path, needs_explicit_caps, stroke_options, stroke_outline},
};
use std::ops::{Mul, Range};

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::{
    canvas::CanvasDimensions,
    types::{PathTransform, StrokeCap, StrokeJoin},
};
use avenger_scenegraph::marks::{
    arc::SceneArcMark, area::SceneAreaMark, group::Clip, image::SceneImageMark,
    line::SceneLineMark, mesh_image::SceneMeshImageMark, path::ScenePathMark, pattern::PatternFill,
    rect::SceneRectMark, rule::SceneRuleMark, symbol::SceneSymbolMark, trail::SceneTrailMark,
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
    path::{builder::BorderRadii, Path, Winding},
};
use wgpu::{
    util::DeviceExt, BindGroup, BindGroupLayout, Device, Extent3d, Queue, RenderPipeline,
    ShaderModule, TextureFormat, VertexBufferLayout,
};

use crate::{
    canvas::MarkContext,
    error::AvengerWgpuError,
    marks::{
        gradient::{to_color_or_gradient_coord, GradientAtlasBuilder},
        image::ImageAtlasBuilder,
        pattern::{PatternRecords, PATTERN_TEXTURE_WIDTH},
    },
    target::PassTarget,
};

#[cfg(feature = "rayon")]
use {crate::par_izip, rayon::prelude::*};

pub const GRADIENT_TEXTURE_CODE: f32 = -1.0;
pub const IMAGE_TEXTURE_CODE: f32 = -2.0;
pub const TEXT_TEXTURE_CODE: f32 = -3.0;
pub const TEXT_TEXTURE_NEAREST_CODE: f32 = -4.0;

const NORMALIZED_SYMBOL_STROKE_WIDTH: f32 = 0.1;
const STENCIL_ATTACHMENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;

/// A mesh's triangle indices, with each triangle wound to face front, so back-face culling keeps
/// it whichever way the mesh lists it.
fn front_facing_indices(positions: &[[f32; 2]], indices: &[u32]) -> Vec<u32> {
    indices
        .chunks_exact(3)
        .flat_map(|triangle| {
            let [a, b, c] = [0, 1, 2].map(|i| positions[triangle[i] as usize]);
            // Front faces turn clockwise on the screen, whose y axis points down.
            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if cross > 0.0 {
                [triangle[0], triangle[2], triangle[1]]
            } else {
                [triangle[0], triangle[1], triangle[2]]
            }
        })
        .collect()
}

pub(crate) fn is_axis_aligned_angle(angle: f32) -> bool {
    let normalized = angle.rem_euclid(360.0);
    (normalized < 0.001)
        || ((normalized - 90.0).abs() < 0.001)
        || ((normalized - 180.0).abs() < 0.001)
        || ((normalized - 270.0).abs() < 0.001)
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MultiUniform {
    pub size: [f32; 2],
    pub scale: f32,
    _pad: [f32; 1],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MultiVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub top_left: [f32; 2],
    pub bottom_right: [f32; 2],
    /// The offset of the fill's pattern record, or 0 for no pattern.
    pub pattern: u32,
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
    1 => Float32x4,     // color
    2 => Float32x2,     // top_left
    3 => Float32x2,     // bottom_right
    4 => Uint32,        // pattern
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
/// reused across any number of `encode_multi_ranges_into` calls. Splitting prepare from
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
    pattern_records: PatternRecords,
    dimensions: CanvasDimensions,
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
        // The chart uniform, and the frame's pattern records.
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
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
            label: Some("chart_uniform_layout"),
        });
        let texture_layout = Self::make_texture_bind_group_layout(device, 1);
        // Text binds a linear sampler and a nearest one, which keeps axis-aligned text sharp.
        let text_layout = Self::make_texture_bind_group_layout(device, 2);
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
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
        );

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
                cull_mode: Some(wgpu::Face::Back),
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
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("srgb_target", f64::from(u8::from(texture_format.is_srgb())))],
                    ..Default::default()
                },
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

    /// The layout of a texture at binding 0 and that many samplers after it.
    fn make_texture_bind_group_layout(device: &Device, samplers: u32) -> BindGroupLayout {
        let texture_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension: wgpu::TextureViewDimension::D2,
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
            },
            count: None,
        };
        let sampler_entries = (1..=samplers).map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            // Filtering, as the texture entry's sample type is filterable.
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        let entries: Vec<_> = std::iter::once(texture_entry)
            .chain(sampler_entries)
            .collect();
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &entries,
            label: Some("texture_bind_group_layout"),
        })
    }
}

impl MultiMarkRenderer {
    #[tracing::instrument(skip_all)]
    pub fn add_mesh_image_mark(
        &mut self,
        mark: &SceneMeshImageMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        // The vertices' texture coordinates place each image on its mesh, and the image branch of
        // the shader doesn't read the bounds.
        let Some([min_x, min_y, max_x, max_y]) = mark.bounds(origin) else {
            return Ok(());
        };
        let top_left = [min_y, min_x];
        let bottom_right = [max_y, max_x];
        // The instances share the mark's clip, and those on one atlas page draw as one batch.
        let clip_indices_range = self.add_clip_path(clip, mark.clip)?;
        let mut batch: Option<MultiMarkBatch> = None;
        for instance in mark.instances.iter().filter(|instance| instance.is_valid()) {
            // An image whose pixels don't match its size draws nothing, as in SVG and PDF.
            let Some(rgba_image) = instance.image.to_image() else {
                continue;
            };
            let (atlas_index, tex_coords) = self.image_atlas_builder.register_image(&rgba_image)?;
            let atlas_width = tex_coords.x1 - tex_coords.x0;
            let atlas_height = tex_coords.y1 - tex_coords.y0;
            let verts = instance
                .positions
                .iter()
                .zip(&instance.uvs)
                .map(|(position, uv)| MultiVertex {
                    color: [
                        IMAGE_TEXTURE_CODE,
                        tex_coords.x0 + uv[0].clamp(0.0, 1.0) * atlas_width,
                        tex_coords.y0 + uv[1].clamp(0.0, 1.0) * atlas_height,
                        0.0,
                    ],
                    position: [position[0] + origin[0], position[1] + origin[1]],
                    top_left,
                    bottom_right,
                    pattern: 0,
                })
                .collect::<Vec<_>>();
            let indices = front_facing_indices(&instance.positions, &instance.indices);
            let start = self.num_indices() as u32;
            let end = start + indices.len() as u32;
            self.verts_inds.push((verts, indices));
            match &mut batch {
                Some(current) if current.image_atlas_index == Some(atlas_index) => {
                    current.indices_range.end = end;
                }
                _ => {
                    self.batches.extend(batch.take());
                    batch = Some(MultiMarkBatch {
                        indices_range: start..end,
                        clip: clip.maybe_clip(mark.clip),
                        clip_indices_range: clip_indices_range.clone(),
                        image_atlas_index: Some(atlas_index),
                        image_smooth: mark.smooth,
                        gradient_atlas_index: None,
                        text_atlas_index: None,
                    });
                }
            }
        }
        self.batches.extend(batch);
        Ok(())
    }

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
                            pattern: 0,
                        },
                        // Lower left
                        MultiVertex {
                            color: [IMAGE_TEXTURE_CODE, tex_coords.x0, tex_coords.y1, 0.0],
                            position: [left, top + height],
                            top_left,
                            bottom_right,
                            pattern: 0,
                        },
                        // Lower right
                        MultiVertex {
                            color: [IMAGE_TEXTURE_CODE, tex_coords.x1, tex_coords.y1, 0.0],
                            position: [left + width, top + height],
                            top_left,
                            bottom_right,
                            pattern: 0,
                        },
                        // Upper right
                        MultiVertex {
                            color: [IMAGE_TEXTURE_CODE, tex_coords.x1, tex_coords.y0, 0.0],
                            position: [left + width, top],
                            top_left,
                            bottom_right,
                            pattern: 0,
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
            },
            gradient_atlas_builder: GradientAtlasBuilder::new(),
            image_atlas_builder: ImageAtlasBuilder::new(),
            pattern_records: PatternRecords::default(),
        }
    }

    pub fn clear(&mut self) {
        self.verts_inds.clear();
        self.clip_verts_inds.clear();
        self.batches.clear();
        self.gradient_atlas_builder = GradientAtlasBuilder::new();
        self.image_atlas_builder = ImageAtlasBuilder::new();
        self.pattern_records = PatternRecords::default();
    }

    pub fn reset_for_frame(&mut self, dimensions: CanvasDimensions) {
        self.clear();
        self.dimensions = dimensions;
        self.uniform = MultiUniform {
            size: dimensions.size,
            scale: dimensions.scale,
            _pad: [0.0],
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
                    fill_pattern: 0,
                },
            );

            // Tesselate fill
            let mut fill_tessellator = FillTessellator::new();
            fill_tessellator.tessellate_path(path, &fill_options(), &mut builder)?;

            let start_index = self.num_clip_indices() as u32;
            self.clip_verts_inds
                .push((buffers.vertices, buffers.indices));
            let end_index = self.num_clip_indices() as u32;
            Ok(Some(start_index..end_index))
        } else {
            Ok(None)
        }
    }

    /// The pattern record of each item's fill, in item order, or 0 for an item without a pattern.
    fn register_patterns<'a>(
        &mut self,
        patterns: impl Iterator<Item = &'a Option<PatternFill>>,
        fills: impl Iterator<Item = &'a ColorOrGradient>,
        gradients: &[Gradient],
        context: &MarkContext,
    ) -> Result<Vec<u32>, AvengerWgpuError> {
        // Items usually share one pattern and fill, so reuse the last item's record.
        let mut last: Option<(&PatternFill, &ColorOrGradient, u32)> = None;
        let mut offsets = Vec::new();
        for (pattern, fill) in patterns.zip(fills) {
            let Some(pattern) = pattern else {
                offsets.push(0);
                continue;
            };
            let offset = match last {
                Some((last_pattern, last_fill, offset))
                    if std::ptr::eq(last_pattern, pattern) && last_fill == fill =>
                {
                    offset
                }
                _ => self.pattern_records.register(
                    pattern,
                    fill,
                    gradients,
                    context.pattern_origin,
                )?,
            };
            last = Some((pattern, fill, offset));
            offsets.push(offset);
        }
        Ok(offsets)
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

        let dashes = mark
            .stroke_dash_iter()
            .map(|iter| iter.map(|dash| Some(dash.as_slice())).collect::<Vec<_>>())
            .unwrap_or_else(|| vec![None; mark.len as usize]);
        let verts_inds = izip!(
            mark.undashed_path_iter(origin),
            mark.stroke_iter(),
            mark.stroke_width_iter(),
            mark.stroke_cap_iter(),
            dashes
        )
        .map(|(path, stroke, width, cap, dash)| {
            tessellate_stroke_path(
                &path,
                stroke,
                &grad_coords,
                *width,
                *cap,
                StrokeJoin::Miter,
                dash,
            )
        })
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

    #[tracing::instrument(skip_all)]
    pub fn add_rect_mark(
        &mut self,
        mark: &SceneRectMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        let (origin, clip) = (context.origin, context.clip);
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);
        let patterns = self.register_patterns(
            mark.fill_pattern_iter(),
            mark.fill_iter(),
            &mark.gradients,
            context,
        )?;

        let verts_inds = if mark.gradients.is_empty()
            && mark.stroke_width.equals_scalar(0.0)
            && mark.corner_radius.equals_scalar(0.0)
        {
            // Handle simple case of no stroke, rounded corners, or gradient. In this case we don't need
            // lyon to perform the tesselation, which saves a bit of time. The contents of this loop are so
            // fast that parallelization doesn't help.
            let mut verts: Vec<MultiVertex> = Vec::with_capacity((mark.len * 4) as usize);
            let mut indicies: Vec<u32> = Vec::with_capacity((mark.len * 6) as usize);

            for (i, x, y, x2, y2, fill, pattern) in izip!(
                0..mark.len,
                mark.x_iter(),
                mark.y_iter(),
                mark.x2_iter(),
                mark.y2_iter(),
                mark.fill_iter(),
                &patterns
            ) {
                let pattern = *pattern;
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
                    pattern,
                });
                verts.push(MultiVertex {
                    position: [x0, y1],
                    color,
                    top_left,
                    bottom_right,
                    pattern,
                });
                verts.push(MultiVertex {
                    position: [x1, y1],
                    color,
                    top_left,
                    bottom_right,
                    pattern,
                });
                verts.push(MultiVertex {
                    position: [x1, y0],
                    color,
                    top_left,
                    bottom_right,
                    pattern,
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
                 corner_radius: &f32,
                 pattern: u32|
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
                    if !path_is_finite(&path) {
                        return Ok((Vec::new(), Vec::new()));
                    }
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
                            fill_pattern: pattern,
                        },
                    );

                    FillTessellator::new().tessellate_path(&path, &fill_options(), &mut builder)?;
                    if *stroke_width > 0.0 {
                        StrokeTessellator::new().tessellate_path(
                            &path,
                            &stroke_options(*stroke_width, StrokeCap::Butt, StrokeJoin::Miter),
                            &mut builder,
                        )?;
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
                        patterns,
                    ).map(|(x, y, x2, y2, fill, stroke, stroke_width, corner_radius, pattern)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                        build_verts_inds(x, &y, &x2, &y2, &fill, &stroke, &stroke_width, &corner_radius, pattern)
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
                        patterns,
                    ).map(|(x, y, x2, y2, fill, stroke, stroke_width, corner_radius, pattern)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                            build_verts_inds(x, y, &x2, &y2, fill, stroke, stroke_width, corner_radius, pattern)
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
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        let (origin, clip) = (context.origin, context.clip);
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);
        let patterns = self.register_patterns(
            mark.fill_pattern_iter(),
            mark.fill_iter(),
            &mark.gradients,
            context,
        )?;

        let build_verts_inds = |path: &lyon::path::Path,
                                fill: &ColorOrGradient,
                                stroke: &ColorOrGradient,
                                pattern: u32|
         -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
            if !path_is_finite(path) {
                return Ok((Vec::new(), Vec::new()));
            }
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
                    fill_pattern: pattern,
                },
            );

            FillTessellator::new().tessellate_path(path, &fill_options(), &mut builder)?;

            // Strokes share other marks' tessellation, which orients square caps on zero-length
            // subpaths, such as a leader's dots.
            if let Some(stroke_width) = mark.stroke_width {
                let (stroke_verts, stroke_indices) = tessellate_stroke_path(
                    path,
                    stroke,
                    &grad_coords,
                    stroke_width,
                    mark.stroke_cap,
                    mark.stroke_join,
                    None,
                )?;
                let offset = buffers.vertices.len() as u32;
                buffers.vertices.extend(stroke_verts);
                buffers
                    .indices
                    .extend(stroke_indices.into_iter().map(|index| index + offset));
            }

            Ok((buffers.vertices, buffers.indices))
        };

        cfg_if::cfg_if! {
            if #[cfg(feature = "rayon")] {
                let verts_inds = par_izip!(
                    mark.transformed_path_iter(origin).collect::<Vec<_>>(),
                    mark.fill_vec(),
                    mark.stroke_vec(),
                    patterns,
                ).map(|(path, fill, stroke, pattern)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                    build_verts_inds(path, &fill, &stroke, pattern)
                }).collect::<Result<Vec<_>, AvengerWgpuError>>()?;
            } else {
                let verts_inds = izip!(
                    mark.transformed_path_iter(origin),
                    mark.fill_iter(),
                    mark.stroke_iter(),
                    patterns,
                ).map(|(path, fill, stroke, pattern)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                    build_verts_inds(&path, &fill, &stroke, pattern)
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
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        let (origin, clip) = (context.origin, context.clip);
        let patterns = self.register_patterns(
            mark.fill_pattern_iter(),
            mark.fill_iter(),
            &mark.gradients,
            context,
        )?;
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
            let fill_options = FillOptions::non_zero().with_tolerance(0.1);

            fill_tessellator.tessellate_path(&path, &fill_options, &mut builder)?;

            // Tesselate stroke
            if stroke_width > 0.0 {
                let mut stroke_tessellator = StrokeTessellator::new();
                let stroke_options = StrokeOptions::default()
                    .with_miter_limit(LYON_SCENE_MITER_LIMIT)
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
                                shape_index: &usize,
                                pattern: u32|
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
                        pattern,
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
                    patterns,
                ).map(|(x, y, fill, size, stroke, angle, shape_index, pattern)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                    build_verts_inds(x, &y, &fill, &size, &stroke, &angle, &shape_index, pattern)
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
                    patterns,
                ).map(|(x, y, fill, size, stroke, angle, shape_index, pattern)| -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                    build_verts_inds(&x, &y, fill, size, stroke, angle, shape_index, pattern)
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

        let path = mark.undashed_path(origin);
        let (verts, indices) = tessellate_stroke_path(
            &path,
            &mark.stroke,
            &grad_coords,
            mark.stroke_width,
            mark.stroke_cap,
            mark.stroke_join,
            mark.stroke_dash.as_deref(),
        )?;

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
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        let (origin, clip) = (context.origin, context.clip);
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);

        let fill_path = mark.transformed_path(origin);
        if !path_is_finite(&fill_path) {
            return Ok(());
        }

        let pattern = self.register_patterns(
            std::iter::once(&mark.fill_pattern),
            std::iter::once(&mark.fill),
            &mark.gradients,
            context,
        )?[0];

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
                fill_pattern: pattern,
            },
        );

        FillTessellator::new().tessellate_path(
            &fill_path,
            &fill_options(),
            &mut buffers_builder,
        )?;

        // The stroke follows the fill's outline, with the area's dashes.
        let (stroke_verts, stroke_indices) = tessellate_stroke_path(
            &fill_path,
            &mark.stroke,
            &grad_coords,
            mark.stroke_width,
            mark.stroke_cap,
            mark.stroke_join,
            mark.stroke_dash.as_deref(),
        )?;
        let offset = buffers.vertices.len() as u32;
        buffers.vertices.extend(stroke_verts);
        buffers
            .indices
            .extend(stroke_indices.into_iter().map(|index| index + offset));

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

        let centerline = mark.transformed_path(origin);
        if !path_is_finite(&centerline) {
            return Ok(());
        }
        // A trail is a variable-width stroke, drawn as its filled outline in the stroke's paint,
        // with gradients on the centerline's bounds.
        let (vertices, indices) = tessellate_fill_path(
            &mark.outline(origin)?,
            &mark.stroke,
            &grad_coords,
            Some(GradientBounds::from_path(&centerline)),
        )?;

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

        self.verts_inds.push((vertices, indices));
        self.batches.push(batch);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub fn add_arc_mark(
        &mut self,
        mark: &SceneArcMark,
        context: &MarkContext,
    ) -> Result<(), AvengerWgpuError> {
        let (origin, clip) = (context.origin, context.clip);
        let (gradient_atlas_index, grad_coords) = self
            .gradient_atlas_builder
            .register_gradients(&mark.gradients);

        let patterns = self.register_patterns(
            mark.fill_pattern_iter(),
            mark.fill_iter(),
            &mark.gradients,
            context,
        )?;

        let verts_inds = izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter(),
            patterns,
        )
        .map(
            |(
                path,
                fill,
                stroke,
                stroke_width,
                pattern,
            )|
             -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
                if !path_is_finite(&path) {
                    return Ok((Vec::new(), Vec::new()));
                }
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
                        fill_pattern: pattern,
                    },
                );

                FillTessellator::new().tessellate_path(
                    &path,
                    &fill_options(),
                    &mut builder,
                )?;
                if *stroke_width > 0.0 {
                    StrokeTessellator::new().tessellate_path(
                        &path,
                        &stroke_options(*stroke_width, StrokeCap::Butt, StrokeJoin::Miter),
                        &mut builder,
                    )?;
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

    fn num_indices(&self) -> usize {
        self.verts_inds.iter().map(|(_, inds)| inds.len()).sum()
    }

    fn num_clip_indices(&self) -> usize {
        self.clip_verts_inds
            .iter()
            .map(|(_, inds)| inds.len())
            .sum()
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
        let pattern_size = self.pattern_records.texture_size();
        let pattern_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Pattern texture"),
            size: pattern_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &pattern_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&self.pattern_records.texture_texels()),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(PATTERN_TEXTURE_WIDTH * 16),
                rows_per_image: Some(pattern_size.height),
            },
            pattern_size,
        );
        let pattern_view = pattern_texture.create_view(&Default::default());
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &resources.uniform_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&pattern_view),
                },
            ],
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
            &[wgpu::FilterMode::Nearest],
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

    /// Bind a batch's gradient, image, and text atlas pages.
    fn set_batch_bind_groups(
        rp: &mut wgpu::RenderPass<'_>,
        prepared: &PreparedMulti,
        text_bind_groups: &[BindGroup],
        batch: &MultiMarkBatch,
    ) {
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
    }

    /// Encode the draws for a set of batch ranges into a caller-provided command encoder, in
    /// order. A run of batches without path clips shares one pass. A path clip opens a pass with
    /// a fresh stencil, which the consecutive batches with the same clip share.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_multi_ranges_into(
        &self,
        mark_encoder: &mut wgpu::CommandEncoder,
        target: PassTarget<'_>,
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

        // A depth-stencil attachment with a fresh stencil, for a pass with a clip or a pattern.
        let fresh_stencil = || {
            depth_view
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
                })
        };

        let mut i = 0usize;
        while i < order.len() {
            let batch = &self.batches[order[i]];
            if batch
                .clip_indices_range
                .as_ref()
                .is_some_and(|range| range.is_empty())
            {
                // A clip path with no area hides the batch.
                i += 1;
                continue;
            }
            if let Some(clip_indices_range) = batch.clip_indices_range.clone() {
                // A pass for a path clip and the batches that share it, with a fresh stencil.
                let mut rp = mark_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Multi Mark Render Pass (stencil)"),
                    color_attachments: &[Some(target.load_attachment())],
                    depth_stencil_attachment: fresh_stencil(),
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                target.set_viewport(&mut rp);
                rp.set_bind_group(0, &prepared.uniform_bind_group, &[]);
                Self::set_batch_bind_groups(&mut rp, prepared, text_bind_groups, batch);
                // Draw the clip shape into the stencil (ref 1), then the marks.
                rp.set_stencil_reference(1);
                rp.set_pipeline(&resources.stencil_pipeline);
                rp.set_vertex_buffer(0, prepared.clip_vertex_buffer.slice(..));
                rp.set_index_buffer(
                    prepared.clip_index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                rp.draw_indexed(clip_indices_range, 0, 0..1);
                rp.set_pipeline(&resources.stencil_render_pipeline);
                rp.set_vertex_buffer(0, prepared.vertex_buffer.slice(..));
                rp.set_index_buffer(prepared.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                while i < order.len() {
                    let next = &self.batches[order[i]];
                    if !Self::shares_clip_pass(batch, next) {
                        break;
                    }
                    Self::set_batch_bind_groups(&mut rp, prepared, text_bind_groups, next);
                    Self::apply_scissor(&mut rp, &next.clip, scale, &target);
                    rp.draw_indexed(next.indices_range.clone(), 0, 0..1);
                    i += 1;
                }
            } else {
                // One pass for a run of non-stencil batches.
                let mut rp = mark_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Multi Mark Render Pass"),
                    color_attachments: &[Some(target.load_attachment())],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                target.set_viewport(&mut rp);
                rp.set_pipeline(&resources.render_pipeline);
                rp.set_bind_group(0, &prepared.uniform_bind_group, &[]);
                rp.set_vertex_buffer(0, prepared.vertex_buffer.slice(..));
                rp.set_index_buffer(prepared.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                while i < order.len() {
                    let batch = &self.batches[order[i]];
                    if batch.clip_indices_range.is_some() {
                        break;
                    }
                    Self::set_batch_bind_groups(&mut rp, prepared, text_bind_groups, batch);
                    Self::apply_scissor(&mut rp, &batch.clip, scale, &target);
                    rp.draw_indexed(batch.indices_range.clone(), 0, 0..1);
                    i += 1;
                }
            }
        }
    }

    /// Whether `next` draws in the stencil pass that `first`'s path clip opened: it has the same
    /// clip, which covers some area.
    fn shares_clip_pass(first: &MultiMarkBatch, next: &MultiMarkBatch) -> bool {
        next.clip_indices_range
            .as_ref()
            .is_some_and(|range| !range.is_empty())
            && next.clip == first.clip
    }

    /// Set the scissor rect for a batch's clip, resetting to the whole attachment for other
    /// clips, so a shared render pass matches a fresh pass for each batch.
    fn apply_scissor(rp: &mut wgpu::RenderPass<'_>, clip: &Clip, scale: f32, target: &PassTarget) {
        let [x, y, width, height] = if let Clip::Rect {
            x,
            y,
            width,
            height,
        } = clip
        {
            target.clip_scissor(*x, *y, *width, *height, scale)
        } else {
            target.full_scissor()
        };
        rp.set_scissor_rect(x, y, width, height);
    }

    /// Prepare the frame's resources and encode every batch into `encoder`.
    pub(crate) fn encode_with_resources(
        &self,
        device: &Device,
        queue: &Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: PassTarget<'_>,
        resources: &MultiMarkRenderResources,
        text_bind_groups: &[BindGroup],
    ) -> Result<(), AvengerWgpuError> {
        let prepared = self.prepare(device, queue, target.extent, resources)?;
        self.encode_multi_ranges_into(
            encoder,
            target,
            resources,
            text_bind_groups,
            &prepared,
            std::slice::from_ref(&(0..self.batches.len())),
        );
        Ok(())
    }

    /// A texture per image, each bound with a sampler per filter: the texture at binding 0 and
    /// the samplers after it.
    pub(crate) fn make_texture_bind_groups(
        device: &Device,
        queue: &Queue,
        layout: &BindGroupLayout,
        size: Extent3d,
        images: &[DynamicImage],
        filters: &[wgpu::FilterMode],
    ) -> Vec<BindGroup> {
        let samplers: Vec<_> = filters
            .iter()
            .map(|&filter| make_sampler(device, filter))
            .collect();
        images
            .iter()
            .map(|image| {
                let view = upload_texture(device, queue, size, image);
                make_texture_bind_group(device, layout, &view, &samplers)
            })
            .collect()
    }

    /// A texture per image, bound once with a linear sampler and once with a nearest one. Both
    /// bind groups read the image's one upload.
    fn make_dual_sampler_texture_bind_groups(
        device: &Device,
        queue: &Queue,
        layout: &BindGroupLayout,
        size: Extent3d,
        images: &[DynamicImage],
    ) -> (Vec<BindGroup>, Vec<BindGroup>) {
        let linear = [make_sampler(device, wgpu::FilterMode::Linear)];
        let nearest = [make_sampler(device, wgpu::FilterMode::Nearest)];
        images
            .iter()
            .map(|image| {
                let view = upload_texture(device, queue, size, image);
                (
                    make_texture_bind_group(device, layout, &view, &linear),
                    make_texture_bind_group(device, layout, &view, &nearest),
                )
            })
            .unzip()
    }
}

fn make_sampler(device: &Device, filter: wgpu::FilterMode) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    })
}

/// Uploads an image to a new texture of the atlas's size.
fn upload_texture(
    device: &Device,
    queue: &Queue,
    size: Extent3d,
    image: &DynamicImage,
) -> wgpu::TextureView {
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
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Binds a texture at binding 0 and the samplers after it.
fn make_texture_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    view: &wgpu::TextureView,
    samplers: &[wgpu::Sampler],
) -> BindGroup {
    let entries: Vec<_> = std::iter::once(wgpu::BindGroupEntry {
        binding: 0,
        resource: wgpu::BindingResource::TextureView(view),
    })
    .chain(
        samplers
            .iter()
            .zip(1..)
            .map(|(sampler, binding)| wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::Sampler(sampler),
            }),
    )
    .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout,
        entries: &entries,
        label: Some("texture_bind_group"),
    })
}

fn tessellate_fill_path(
    path: &Path,
    fill: &ColorOrGradient,
    grad_coords: &[f32],
    gradient_bounds: Option<GradientBounds>,
) -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
    if !path_is_finite(path) {
        return Ok((Vec::new(), Vec::new()));
    }
    let bbox = gradient_bounds.unwrap_or_else(|| GradientBounds::from_path(path));
    let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
    let mut builder = BuffersBuilder::new(
        &mut buffers,
        VertexPositions {
            fill: to_color_or_gradient_coord(fill, grad_coords),
            stroke: [0.0, 0.0, 0.0, 0.0],
            top_left: bbox.min,
            bottom_right: bbox.max,
            fill_pattern: 0,
        },
    );

    FillTessellator::new().tessellate_path(path, &fill_options(), &mut builder)?;
    Ok((buffers.vertices, buffers.indices))
}

/// Tessellates a stroke, with gradients on the bounds of its undashed centerline.
fn tessellate_stroke_path(
    path: &Path,
    stroke: &ColorOrGradient,
    grad_coords: &[f32],
    stroke_width: f32,
    stroke_cap: StrokeCap,
    stroke_join: StrokeJoin,
    stroke_dash: Option<&[f32]>,
) -> Result<(Vec<MultiVertex>, Vec<u32>), AvengerWgpuError> {
    if !(stroke_width > 0.0 && stroke_width.is_finite() && path_is_finite(path)) {
        return Ok((Vec::new(), Vec::new()));
    }

    let bbox = GradientBounds::from_path(path);
    // Overlapping triangles with the same opaque color do not accumulate opacity.
    let opaque = matches!(stroke, ColorOrGradient::Color(color) if color[3] == 1.0);
    if needs_explicit_caps(path, stroke_dash, stroke_cap) || (stroke_dash.is_some() && !opaque) {
        let outline = stroke_outline(path, stroke_dash, stroke_width, stroke_cap, stroke_join)?;
        return tessellate_fill_path(&outline, stroke, grad_coords, Some(bbox));
    }
    let dashed_path = stroke_dash.map(|dash| dash_path(path, dash));
    let path = dashed_path.as_ref().unwrap_or(path);
    let mut buffers: VertexBuffers<MultiVertex, u32> = VertexBuffers::new();
    let mut builder = BuffersBuilder::new(
        &mut buffers,
        VertexPositions {
            fill: [0.0, 0.0, 0.0, 0.0],
            stroke: to_color_or_gradient_coord(stroke, grad_coords),
            top_left: bbox.min,
            bottom_right: bbox.max,
            fill_pattern: 0,
        },
    );

    StrokeTessellator::new().tessellate_path(
        path,
        &stroke_options(stroke_width, stroke_cap, stroke_join),
        &mut builder,
    )?;
    Ok((buffers.vertices, buffers.indices))
}

/// The options that tessellate a scene fill, which uses the nonzero rule.
fn fill_options() -> FillOptions {
    FillOptions::non_zero().with_tolerance(0.05)
}

pub struct VertexPositions {
    fill: [f32; 4],
    stroke: [f32; 4],
    top_left: [f32; 2],
    bottom_right: [f32; 2],
    /// The fill's pattern record, or 0 for no pattern.
    fill_pattern: u32,
}

impl FillVertexConstructor<MultiVertex> for VertexPositions {
    fn new_vertex(&mut self, vertex: FillVertex) -> MultiVertex {
        MultiVertex {
            position: [vertex.position().x, vertex.position().y],
            color: self.fill,
            top_left: self.top_left,
            bottom_right: self.bottom_right,
            pattern: self.fill_pattern,
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
            pattern: 0,
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
        fill_pattern: u32,
    ) -> MultiVertex {
        let angle = Angle::degrees(angle);
        let absolue_scale = size.sqrt();
        let relative_scale = absolue_scale / self.scale;

        // Scale
        let mut transform = PathTransform::scale(relative_scale, relative_scale);

        // Compute adjustment factor for stroke vertices to compensate for scaling and
        // achieve the correct final line width.
        let (color, pattern) = if let Some(normal) = self.normal {
            let scaled_line_width = relative_scale * NORMALIZED_SYMBOL_STROKE_WIDTH;
            let line_width_adjustment = normal.mul((line_width - scaled_line_width) / 2.0);
            transform = transform.then_translate(line_width_adjustment);
            (stroke, 0)
        } else {
            (fill, fill_pattern)
        };

        // Rotate then Translate
        transform = transform
            .then_rotate(angle)
            .then_translate(Vector2D::new(x, y));
        let position = transform.transform_point(self.position);

        // Symbols place gradients and mark-anchored patterns on their nominal square, whatever
        // their shape and rotation.
        let bounds = GradientBounds::symbol([x, y], size);
        MultiVertex {
            position: position.to_array(),
            color,
            top_left: bounds.min,
            bottom_right: bounds.max,
            pattern,
        }
    }
}

#[cfg(test)]
mod tests {
    use avenger_common::value::ScalarOrArray;
    use avenger_scenegraph::marks::{
        pattern::{PatternValidationError, StripeLayer},
        rect::SceneRectMark,
    };

    use super::*;

    #[test]
    fn invalid_patterns_are_errors() {
        let mut renderer = MultiMarkRenderer::new(CanvasDimensions {
            size: [40.0, 30.0],
            scale: 1.0,
        });
        let mark = SceneRectMark {
            len: 1,
            x: ScalarOrArray::new_scalar(4.0),
            y: ScalarOrArray::new_scalar(4.0),
            width: Some(ScalarOrArray::new_scalar(24.0)),
            height: Some(ScalarOrArray::new_scalar(16.0)),
            fill_pattern: PatternFill {
                layers: vec![StripeLayer::new(0.0, 0.0, 2.0).into()],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        let context = MarkContext {
            origin: [0.0, 0.0],
            clip: &Clip::None,
            pattern_origin: [0.0, 0.0],
        };

        let err = renderer.add_rect_mark(&mark, &context).unwrap_err();

        assert!(matches!(
            err,
            AvengerWgpuError::InvalidPattern(PatternValidationError::NonPositive("stripe.spacing"))
        ));
    }
}
