use crate::canvas::CanvasDimensions;
use crate::error::AvengerWgpuError;
use crate::marks::basic_mark::{BasicMarkBatch, BasicMarkShader};
use crate::marks::gradient::{build_gradients_image, to_color_or_gradient_coord};
use avenger::marks::area::{AreaMark, AreaOrientation};
use avenger::marks::group::GroupBounds;
use avenger::marks::line::LineMark;
use avenger::marks::path::PathMark;
use avenger::marks::trail::TrailMark;
use avenger::marks::value::{StrokeCap, StrokeJoin};
use itertools::izip;
use lyon::algorithms::aabb::bounding_box;
use lyon::algorithms::measure::{PathMeasurements, PathSampler, SampleType};
use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, StrokeOptions,
    StrokeTessellator, StrokeVertex, StrokeVertexConstructor, VertexBuffers,
};
use lyon::path::builder::WithSvg;
use lyon::path::path::BuilderImpl;
use lyon::path::{AttributeIndex, LineCap, LineJoin, Path};
use wgpu::{Extent3d, VertexBufferLayout};

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PathUniform {
    pub size: [f32; 2],
    pub origin: [f32; 2],
    pub group_size: [f32; 2],
    pub scale: f32,
    pub clip: f32,
}

impl PathUniform {
    pub fn new(dimensions: CanvasDimensions, group_bounds: GroupBounds, clip: bool) -> Self {
        Self {
            size: dimensions.size,
            scale: dimensions.scale,
            origin: [group_bounds.x, group_bounds.y],
            group_size: [
                group_bounds.width.unwrap_or(0.0),
                group_bounds.height.unwrap_or(0.0),
            ],
            clip: if clip && group_bounds.width.is_some() && group_bounds.height.is_some() {
                1.0
            } else {
                0.0
            },
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PathVertex {
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

impl PathVertex {
    pub fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<PathVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }
    }
}

pub struct PathShader {
    verts: Vec<PathVertex>,
    indices: Vec<u16>,
    uniform: PathUniform,
    batches: Vec<BasicMarkBatch>,
    texture_size: Extent3d,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl PathShader {
    pub fn from_path_mark(
        mark: &PathMark,
        dimensions: CanvasDimensions,
        group_bounds: GroupBounds,
    ) -> Result<Self, AvengerWgpuError> {
        let (gradients_image, texture_size) = build_gradients_image(&mark.gradients);

        let mut verts: Vec<PathVertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();

        // Here, add style info to PathVertex
        for (path, fill, stroke, transform) in izip!(
            mark.path_iter(),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.transform_iter(),
        ) {
            // Apply transform to path
            let path = path.clone().transformed(transform);
            let bbox = bounding_box(&path);

            // Create vertex/index buffer builder
            let mut buffers: VertexBuffers<PathVertex, u16> = VertexBuffers::new();
            let mut builder = BuffersBuilder::new(
                &mut buffers,
                VertexPositions {
                    fill: to_color_or_gradient_coord(fill, texture_size),
                    stroke: to_color_or_gradient_coord(stroke, texture_size),
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

            let index_offset = verts.len() as u16;
            verts.extend(buffers.vertices);
            indices.extend(buffers.indices.into_iter().map(|i| i + index_offset));
        }

        let indices_range = 0..indices.len() as u32;
        Ok(Self {
            verts,
            indices,
            uniform: PathUniform::new(dimensions, group_bounds, mark.clip),
            batches: vec![BasicMarkBatch {
                indices_range,
                image: gradients_image,
            }],
            texture_size,
            shader: format!(
                "{}\n{}",
                include_str!("path.wgsl"),
                include_str!("gradient.wgsl")
            ),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }

    pub fn from_area_mark(
        mark: &AreaMark,
        dimensions: CanvasDimensions,
        group_bounds: GroupBounds,
    ) -> Result<Self, AvengerWgpuError> {
        // Handle gradients:
        let (gradients_image, texture_size) = build_gradients_image(&mark.gradients);

        let mut path_builder = lyon::path::Path::builder().with_svg();
        let mut tail: Vec<(f32, f32)> = Vec::new();

        fn close_area(b: &mut WithSvg<BuilderImpl>, tail: &mut Vec<(f32, f32)>) {
            if tail.is_empty() {
                return;
            }
            for (x, y) in tail.iter().rev() {
                b.line_to(lyon::geom::point(*x, *y));
            }

            tail.clear();
            b.close();
        }

        if mark.orientation == AreaOrientation::Vertical {
            for (x, y, y2, defined) in izip!(
                mark.x_iter(),
                mark.y_iter(),
                mark.y2_iter(),
                mark.defined_iter(),
            ) {
                if *defined {
                    if !tail.is_empty() {
                        // Continue path
                        path_builder.line_to(lyon::geom::point(*x, *y));
                    } else {
                        // New path
                        path_builder.move_to(lyon::geom::point(*x, *y));
                    }
                    tail.push((*x, *y2));
                } else {
                    close_area(&mut path_builder, &mut tail);
                }
            }
        } else {
            for (y, x, x2, defined) in izip!(
                mark.y_iter(),
                mark.x_iter(),
                mark.x2_iter(),
                mark.defined_iter(),
            ) {
                if *defined {
                    if !tail.is_empty() {
                        // Continue path
                        path_builder.line_to(lyon::geom::point(*x, *y));
                    } else {
                        // New path
                        path_builder.move_to(lyon::geom::point(*x, *y));
                    }
                    tail.push((*x2, *y));
                } else {
                    close_area(&mut path_builder, &mut tail);
                }
            }
        }

        close_area(&mut path_builder, &mut tail);
        let path = path_builder.build();
        let bbox = bounding_box(&path);

        // Create vertex/index buffer builder
        let mut buffers: VertexBuffers<PathVertex, u16> = VertexBuffers::new();
        let mut buffers_builder = BuffersBuilder::new(
            &mut buffers,
            VertexPositions {
                fill: to_color_or_gradient_coord(&mark.fill, texture_size),
                stroke: to_color_or_gradient_coord(&mark.stroke, texture_size),
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

        let indices_range = 0..buffers.indices.len() as u32;
        Ok(Self {
            verts: buffers.vertices,
            indices: buffers.indices,
            uniform: PathUniform::new(dimensions, group_bounds, mark.clip),
            batches: vec![BasicMarkBatch {
                indices_range,
                image: gradients_image,
            }],
            texture_size,
            shader: format!(
                "{}\n{}",
                include_str!("path.wgsl"),
                include_str!("gradient.wgsl")
            ),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }

    pub fn from_line_mark(
        mark: &LineMark,
        dimensions: CanvasDimensions,
        group_bounds: GroupBounds,
    ) -> Result<Self, AvengerWgpuError> {
        let (gradients_image, texture_size) = build_gradients_image(&mark.gradients);
        let mut defined_paths: Vec<Path> = Vec::new();

        // Build path for each defined line segment
        let mut path_builder = lyon::path::Path::builder().with_svg();
        let mut path_len = 0;
        for (x, y, defined) in izip!(mark.x_iter(), mark.y_iter(), mark.defined_iter()) {
            if *defined {
                if path_len > 0 {
                    // Continue path
                    path_builder.line_to(lyon::geom::point(*x, *y));
                } else {
                    // New path
                    path_builder.move_to(lyon::geom::point(*x, *y));
                }
                path_len += 1;
            } else {
                if path_len == 1 {
                    // Finishing single point line. Add extra point at the same location
                    // so that stroke caps are drawn
                    path_builder.close();
                }
                defined_paths.push(path_builder.build());
                path_builder = lyon::path::Path::builder().with_svg();
                path_len = 0;
            }
        }
        defined_paths.push(path_builder.build());

        let defined_paths = if let Some(stroke_dash) = &mark.stroke_dash {
            // Create new paths with dashing
            let mut dashed_paths: Vec<Path> = Vec::new();
            for path in defined_paths.iter() {
                let mut dash_path_builder = lyon::path::Path::builder();
                let path_measurements = PathMeasurements::from_path(path, 0.1);
                let mut sampler =
                    PathSampler::new(&path_measurements, path, &(), SampleType::Distance);

                // Next index into stroke_dash array
                let mut dash_idx = 0;

                // Distance along line from (x0,y0) to (x1,y1) where the next dash will start
                let mut start_dash_dist: f32 = 0.0;

                // Total length of line
                let line_len = sampler.length();

                // Whether the next dash length represents a drawn dash (draw == true)
                // or a gap (draw == false)
                let mut draw = true;

                while start_dash_dist < line_len {
                    let end_dash_dist = if start_dash_dist + stroke_dash[dash_idx] >= line_len {
                        // The final dash/gap should be truncated to the end of the line
                        line_len
                    } else {
                        // The dash/gap fits entirely in the rule
                        start_dash_dist + stroke_dash[dash_idx]
                    };

                    if draw {
                        sampler.split_range(start_dash_dist..end_dash_dist, &mut dash_path_builder);
                    }

                    // update start dist for next dash/gap
                    start_dash_dist = end_dash_dist;

                    // increment index and cycle back to start of start of dash array
                    dash_idx = (dash_idx + 1) % stroke_dash.len();

                    // Alternate between drawn dash and gap
                    draw = !draw;
                }
                dashed_paths.push(dash_path_builder.build())
            }
            dashed_paths
        } else {
            defined_paths
        };

        let mut verts: Vec<PathVertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();

        for path in &defined_paths {
            let bbox = bounding_box(path);
            // Create vertex/index buffer builder
            let mut buffers: VertexBuffers<PathVertex, u16> = VertexBuffers::new();
            let mut buffers_builder = BuffersBuilder::new(
                &mut buffers,
                VertexPositions {
                    fill: [0.0, 0.0, 0.0, 0.0],
                    stroke: to_color_or_gradient_coord(&mark.stroke, texture_size),
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
            stroke_tessellator.tessellate_path(path, &stroke_options, &mut buffers_builder)?;

            let index_offset = verts.len() as u16;
            verts.extend(buffers.vertices);
            indices.extend(buffers.indices.into_iter().map(|i| i + index_offset));
        }

        let indices_range = 0..indices.len() as u32;
        Ok(Self {
            verts,
            indices,
            uniform: PathUniform::new(dimensions, group_bounds, mark.clip),
            batches: vec![BasicMarkBatch {
                indices_range,
                image: gradients_image,
            }],
            texture_size,
            shader: format!(
                "{}\n{}",
                include_str!("path.wgsl"),
                include_str!("gradient.wgsl")
            ),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }

    pub fn from_trail_mark(
        mark: &TrailMark,
        dimensions: CanvasDimensions,
        group_bounds: GroupBounds,
    ) -> Result<Self, AvengerWgpuError> {
        let (gradients_image, texture_size) = build_gradients_image(&mark.gradients);

        let size_idx: AttributeIndex = 0;
        let mut path_builder = lyon::path::Path::builder_with_attributes(1);
        let mut path_len = 0;
        for (x, y, size, defined) in izip!(
            mark.x_iter(),
            mark.y_iter(),
            mark.size_iter(),
            mark.defined_iter()
        ) {
            if *defined {
                if path_len > 0 {
                    // Continue path
                    path_builder.line_to(lyon::geom::point(*x, *y), &[*size]);
                } else {
                    // New path
                    path_builder.begin(lyon::geom::point(*x, *y), &[*size]);
                }
                path_len += 1;
            } else {
                if path_len == 1 {
                    // Finishing single point line. Add extra point at the same location
                    // so that stroke caps are drawn
                    path_builder.end(true);
                } else {
                    path_builder.end(false);
                }
                path_len = 0;
            }
        }
        path_builder.end(false);

        let path = path_builder.build();
        let bbox = bounding_box(&path);

        // Create vertex/index buffer builder
        let mut buffers: VertexBuffers<PathVertex, u16> = VertexBuffers::new();
        let mut buffers_builder = BuffersBuilder::new(
            &mut buffers,
            VertexPositions {
                fill: [0.0, 0.0, 0.0, 0.0],
                stroke: to_color_or_gradient_coord(&mark.stroke, texture_size),
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
            .with_variable_line_width(size_idx);
        stroke_tessellator.tessellate_path(&path, &stroke_options, &mut buffers_builder)?;

        let indices_range = 0..buffers.indices.len() as u32;
        Ok(Self {
            verts: buffers.vertices,
            indices: buffers.indices,
            uniform: PathUniform::new(dimensions, group_bounds, mark.clip),
            batches: vec![BasicMarkBatch {
                indices_range,
                image: gradients_image,
            }],
            texture_size,
            shader: format!(
                "{}\n{}",
                include_str!("path.wgsl"),
                include_str!("gradient.wgsl")
            ),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }
}

impl BasicMarkShader for PathShader {
    type Vertex = PathVertex;
    type Uniform = PathUniform;

    fn verts(&self) -> &[Self::Vertex] {
        self.verts.as_slice()
    }

    fn indices(&self) -> &[u16] {
        self.indices.as_slice()
    }

    fn uniform(&self) -> Self::Uniform {
        self.uniform
    }

    fn batches(&self) -> &[BasicMarkBatch] {
        self.batches.as_slice()
    }

    fn texture_size(&self) -> Extent3d {
        self.texture_size
    }

    fn shader(&self) -> &str {
        self.shader.as_str()
    }

    fn vertex_entry_point(&self) -> &str {
        self.vertex_entry_point.as_str()
    }

    fn fragment_entry_point(&self) -> &str {
        self.fragment_entry_point.as_str()
    }

    fn vertex_desc(&self) -> VertexBufferLayout<'static> {
        PathVertex::desc()
    }
}

pub struct VertexPositions {
    fill: [f32; 4],
    stroke: [f32; 4],
    top_left: [f32; 2],
    bottom_right: [f32; 2],
}

impl FillVertexConstructor<PathVertex> for VertexPositions {
    fn new_vertex(&mut self, vertex: FillVertex) -> PathVertex {
        PathVertex {
            position: [vertex.position().x, vertex.position().y],
            color: self.fill,
            top_left: self.top_left,
            bottom_right: self.bottom_right,
        }
    }
}

impl StrokeVertexConstructor<PathVertex> for VertexPositions {
    fn new_vertex(&mut self, vertex: StrokeVertex) -> PathVertex {
        PathVertex {
            position: [vertex.position().x, vertex.position().y],
            color: self.stroke,
            top_left: self.top_left,
            bottom_right: self.bottom_right,
        }
    }
}
