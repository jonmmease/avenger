use crate::canvas::TextBuildCtor;
use crate::error::AvengerWgpuError;

use crate::marks::gradient::{to_color_or_gradient_coord, GradientAtlasBuilder};
use crate::marks::image::ImageAtlasBuilder;
use avenger_common::canvas::CanvasDimensions;
use avenger_common::types::{ColorOrGradient, PathTransform, StrokeCap, StrokeJoin};
use avenger_scenegraph::marks::area::SceneAreaMark;
use avenger_scenegraph::marks::image::SceneImageMark;
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::rule::SceneRuleMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::marks::trail::SceneTrailMark;
use etagere::euclid::UnknownUnit;
use image::DynamicImage;
use itertools::izip;
use lyon::algorithms::aabb::bounding_box;
use lyon::geom::euclid::{Point2D, Vector2D};
use lyon::geom::{Angle, Box2D};
use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, LineCap,
    LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex, StrokeVertexConstructor,
    VertexBuffers,
};
use lyon::path::builder::BorderRadii;
use lyon::path::Winding;
use std::ops::{Mul, Range};

use wgpu::util::DeviceExt;
use wgpu::{
    BindGroup, BindGroupLayout, CommandBuffer, Device, Extent3d, Queue, TextureFormat, TextureView,
    VertexBufferLayout,
};

// Import rayon prelude as required by par_izip.
use crate::marks::text::{TextAtlasBuilderTrait, TextInstance};

use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::text::SceneTextMark;

#[cfg(feature = "rayon")]
use {crate::par_izip, rayon::prelude::*};

pub const GRADIENT_TEXTURE_CODE: f32 = -1.0;
pub const IMAGE_TEXTURE_CODE: f32 = -2.0;
pub const TEXT_TEXTURE_CODE: f32 = -3.0;

const NORMALIZED_SYMBOL_STROKE_WIDTH: f32 = 0.1;

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
    pub gradient_atlas_index: Option<usize>,
    pub text_atlas_index: Option<usize>,
}

pub struct MultiMarkRenderer {
    verts_inds: Vec<(Vec<MultiVertex>, Vec<u32>)>,
    clip_verts_inds: Vec<(Vec<MultiVertex>, Vec<u32>)>,
    batches: Vec<MultiMarkBatch>,
    uniform: MultiUniform,
    gradient_atlas_builder: GradientAtlasBuilder,
    image_atlas_builder: ImageAtlasBuilder,
    text_atlas_builder: Box<dyn TextAtlasBuilderTrait>,
    dimensions: CanvasDimensions,
}

impl MultiMarkRenderer {
    pub fn new(
        dimensions: CanvasDimensions,
        text_atlas_builder_ctor: Option<TextBuildCtor>,
    ) -> Self {
        let text_atlas_builder = if let Some(text_atlas_builder_ctor) = text_atlas_builder_ctor {
            text_atlas_builder_ctor()
        } else {
            cfg_if::cfg_if! {
                if #[cfg(feature = "cosmic-text")] {
                    use crate::marks::text::TextAtlasBuilder;
                    use std::sync::Arc;
                    let inner_text_atlas_builder: Box<dyn TextAtlasBuilderTrait> = Box::new(TextAtlasBuilder::new(Arc::new(
                        avenger_text::rasterization::cosmic::CosmicTextRasterizer::<crate::marks::text::GlyphBBoxAndAtlasCoords>::new())
                    ));
                } else if #[cfg(target_arch = "wasm32")] {
                    use crate::marks::text::TextAtlasBuilder;
                    use std::sync::Arc;
                    let inner_text_atlas_builder: Box<dyn TextAtlasBuilderTrait> = Box::new(TextAtlasBuilder::new(Arc::new(
                        avenger_text::rasterization::html_canvas::HtmlCanvasTextRasterizer::<crate::marks::text::GlyphBBoxAndAtlasCoords>::new())
                    ));
                } else {
                    use crate::marks::text::NullTextAtlasBuilder;
                    let inner_text_atlas_builder: Box<dyn TextAtlasBuilderTrait> = Box::new(NullTextAtlasBuilder);
                }
            };
            inner_text_atlas_builder
        };

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
            text_atlas_builder,
        }
    }

    pub fn clear(&mut self) {
        self.verts_inds.clear();
        self.clip_verts_inds.clear();
        self.batches.clear();
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

        let path = mark.transformed_path(origin);
        let bbox = bounding_box(&path);

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
        fill_tessellator.tessellate_path(&path, &fill_options, &mut buffers_builder)?;

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
            stroke_tessellator.tessellate_path(&path, &stroke_options, &mut buffers_builder)?;
        }

        let start_ind = self.num_indices();
        let indices_range = (start_ind as u32)..((start_ind + buffers.indices.len()) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
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

        let start_ind = self.num_indices();
        let indices_range = (start_ind as u32)..((start_ind + buffers.indices.len()) as u32);

        let batch = MultiMarkBatch {
            indices_range,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
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
            gradient_atlas_index,
            text_atlas_index: None,
        };

        self.verts_inds.extend(verts_inds);
        self.batches.push(batch);
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

    #[tracing::instrument(skip_all)]
    pub fn add_text_mark(
        &mut self,
        mark: &SceneTextMark,
        origin: [f32; 2],
        clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        let registrations = izip!(
            mark.text_iter(),
            mark.x_iter(),
            mark.y_iter(),
            mark.color_iter(),
            mark.align_iter(),
            mark.angle_iter(),
            mark.baseline_iter(),
            mark.font_iter(),
            mark.font_size_iter(),
            mark.font_weight_iter(),
            mark.font_style_iter(),
            mark.limit_iter(),
        )
        .map(
            |(
                text,
                x,
                y,
                color,
                align,
                angle,
                baseline,
                font,
                font_size,
                font_weight,
                font_style,
                limit,
            )| {
                let instance = TextInstance {
                    text,
                    position: [*x + origin[0], *y + origin[1]],
                    color: &color.color_or_transparent(),
                    align,
                    angle: *angle,
                    baseline,
                    font,
                    font_size: *font_size,
                    font_weight,
                    font_style,
                    limit: *limit,
                };
                self.text_atlas_builder
                    .register_text(instance, self.dimensions)
            },
        )
        .collect::<Result<Vec<_>, AvengerWgpuError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        // Construct batches, one batch per text atlas index
        let start_ind = self.num_indices() as u32;
        let mut next_batch = MultiMarkBatch {
            indices_range: start_ind..start_ind,
            clip: clip.maybe_clip(mark.clip),
            clip_indices_range: self.add_clip_path(clip, mark.clip)?,
            image_atlas_index: None,
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
                    clip: clip.maybe_clip(mark.clip),
                    clip_indices_range: self.add_clip_path(clip, mark.clip)?,
                    image_atlas_index: None,
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

    #[tracing::instrument(skip_all)]
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
        let (grad_texture_size, grad_images) = self.gradient_atlas_builder.build();
        let (gradient_layout, gradient_texture_bind_groups) = Self::make_texture_bind_groups(
            device,
            queue,
            grad_texture_size,
            &grad_images,
            wgpu::FilterMode::Nearest,
            wgpu::FilterMode::Nearest,
        );

        // Image Textures
        let (image_texture_size, image_images) = self.image_atlas_builder.build();
        let (image_layout, image_texture_bind_groups) = Self::make_texture_bind_groups(
            device,
            queue,
            image_texture_size,
            &image_images,
            wgpu::FilterMode::Linear,
            wgpu::FilterMode::Linear,
        );

        // Text Textures
        let (text_texture_size, text_images) = self.text_atlas_builder.build();
        let (text_layout, text_texture_bind_groups) = Self::make_texture_bind_groups(
            device,
            queue,
            text_texture_size,
            &text_images,
            wgpu::FilterMode::Linear,
            wgpu::FilterMode::Linear,
        );

        // Shaders
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("multi.wgsl").into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[
                    &uniform_layout,
                    &gradient_layout,
                    &image_layout,
                    &text_layout,
                ],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[MultiVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Stencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState {
                    front: wgpu::StencilFaceState {
                        // Draw pixel if stencil reference value is less than or equal to stencil value
                        compare: wgpu::CompareFunction::LessEqual,
                        ..Default::default()
                    },
                    back: wgpu::StencilFaceState::IGNORE,
                    read_mask: !0,
                    write_mask: !0,
                },
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let stencil_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[MultiVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: texture_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Stencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState {
                    front: wgpu::StencilFaceState {
                        compare: wgpu::CompareFunction::Always,
                        pass_op: wgpu::StencilOperation::Replace,
                        ..Default::default()
                    },
                    back: wgpu::StencilFaceState::IGNORE,
                    read_mask: !0,
                    write_mask: !0,
                },
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let stencil_buffer = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Stencil buffer"),
            size: Extent3d {
                width: self.dimensions.to_physical_width(),
                height: self.dimensions.to_physical_height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Stencil8,
            view_formats: &[],
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
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

        // Create command encoder for marks
        let mut mark_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Multi Mark Render Encoder"),
        });

        // Render batches
        {
            let depth_view = stencil_buffer.create_view(&Default::default());
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
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: if cfg!(feature = "deno") {
                        // depth_ops shouldn't be needed, but setting to None results in validation
                        // error in Deno. However, setting it to the below causes a validation error
                        // in Chrome.
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
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            if vertex_buffer.size() > 0 {
                render_pass.set_pipeline(&render_pipeline);
                render_pass.set_bind_group(0, &uniform_bind_group, &[]);
                let mut last_grad_ind = 0;
                let mut last_img_ind = 0;
                let mut last_text_ind = 0;
                let mut stencil_index: u32 = 1;
                render_pass.set_bind_group(1, &gradient_texture_bind_groups[last_grad_ind], &[]);
                render_pass.set_bind_group(2, &image_texture_bind_groups[last_img_ind], &[]);
                render_pass.set_bind_group(3, &text_texture_bind_groups[last_img_ind], &[]);
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);

                // Initialze textures with first entry
                for batch in &self.batches {
                    if let Some(clip_inds_range) = &batch.clip_indices_range {
                        render_pass.set_stencil_reference(stencil_index);
                        render_pass.set_pipeline(&stencil_pipeline);
                        render_pass.set_vertex_buffer(0, clip_vertex_buffer.slice(..));
                        render_pass.set_index_buffer(
                            clip_index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        render_pass.draw_indexed(clip_inds_range.clone(), 0, 0..1);

                        // Restore buffers
                        render_pass.set_pipeline(&render_pipeline);
                        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                        render_pass
                            .set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);

                        // increment stencil index for next draw
                        stencil_index += 1;
                    } else {
                        // Set stencil reference back to zero so that everything is drawn
                        render_pass.set_stencil_reference(0);
                    }

                    // Update scissors
                    if let Clip::Rect {
                        x,
                        y,
                        width,
                        height,
                    } = batch.clip
                    {
                        // Convert to physical coordinates
                        let physical_x = (x * self.uniform.scale) as u32;
                        let physical_y = (y * self.uniform.scale) as u32;
                        let physical_width = (width * self.uniform.scale) as u32;
                        let physical_height = (height * self.uniform.scale) as u32;

                        // Clamp to canvas dimensions
                        let canvas_width = self.dimensions.to_physical_width();
                        let canvas_height = self.dimensions.to_physical_height();

                        let clamped_x = physical_x.min(canvas_width);
                        let clamped_y = physical_y.min(canvas_height);
                        let clamped_width = physical_width.min(canvas_width - clamped_x);
                        let clamped_height = physical_height.min(canvas_height - clamped_y);

                        // Set scissors rect with clamped values
                        render_pass.set_scissor_rect(
                            clamped_x,
                            clamped_y,
                            clamped_width,
                            clamped_height,
                        );
                    } else {
                        // Clear scissors rect
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
                            render_pass.set_bind_group(
                                1,
                                &gradient_texture_bind_groups[grad_ind],
                                &[],
                            );
                            last_grad_ind = grad_ind;
                        }
                    }

                    if let Some(img_ind) = batch.image_atlas_index {
                        if img_ind != last_img_ind {
                            render_pass.set_bind_group(2, &image_texture_bind_groups[img_ind], &[]);
                        }
                        last_img_ind = img_ind;
                    }

                    if let Some(text_ind) = batch.text_atlas_index {
                        if text_ind != last_text_ind {
                            render_pass.set_bind_group(3, &text_texture_bind_groups[text_ind], &[]);
                        }
                        last_text_ind = text_ind;
                    }

                    // draw inds
                    render_pass.draw_indexed(batch.indices_range.clone(), 0, 0..1);
                }
            }
        }

        mark_encoder.finish()
    }

    fn make_texture_bind_groups(
        device: &Device,
        queue: &Queue,
        size: Extent3d,
        images: &[DynamicImage],
        mag_filter: wgpu::FilterMode,
        min_filter: wgpu::FilterMode,
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
