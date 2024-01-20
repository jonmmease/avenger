use crate::error::Sg2dWgpuError;
use crate::marks::basic_mark::BasicMarkShader;
use itertools::izip;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, StrokeOptions,
    StrokeTessellator, StrokeVertex, StrokeVertexConstructor, VertexBuffers,
};
use lyon::path::builder::WithSvg;
use lyon::path::path::BuilderImpl;
use lyon::path::{AttributeIndex, LineCap, LineJoin};
use sg2d::marks::area::{AreaMark, AreaOrientation};
use sg2d::marks::line::LineMark;
use sg2d::marks::path::PathMark;
use sg2d::marks::trail::TrailMark;
use sg2d::value::{StrokeCap, StrokeJoin};
use wgpu::VertexBufferLayout;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PathVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
    1 => Float32x4,     // color
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
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl PathShader {
    pub fn from_path_mark(mark: &PathMark) -> Result<Self, Sg2dWgpuError> {
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

            // Create vertex/index buffer builder
            let mut buffers: VertexBuffers<PathVertex, u16> = VertexBuffers::new();
            let mut builder = BuffersBuilder::new(
                &mut buffers,
                VertexPositions {
                    fill: *fill,
                    stroke: *stroke,
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

        Ok(Self {
            verts,
            indices,
            shader: include_str!("path.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }

    pub fn from_area_mark(mark: &AreaMark) -> Result<Self, Sg2dWgpuError> {
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

        // Create vertex/index buffer builder
        let mut buffers: VertexBuffers<PathVertex, u16> = VertexBuffers::new();
        let mut buffers_builder = BuffersBuilder::new(
            &mut buffers,
            VertexPositions {
                fill: mark.fill,
                stroke: mark.stroke,
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

        Ok(Self {
            verts: buffers.vertices,
            indices: buffers.indices,
            shader: include_str!("path.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }

    pub fn from_line_mark(mark: &LineMark) -> Result<Self, Sg2dWgpuError> {
        // Build path
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
                path_len = 0;
            }
        }

        let path = path_builder.build();

        // Create vertex/index buffer builder
        let mut buffers: VertexBuffers<PathVertex, u16> = VertexBuffers::new();
        let mut buffers_builder = BuffersBuilder::new(
            &mut buffers,
            VertexPositions {
                fill: [0.0, 0.0, 0.0, 0.0],
                stroke: mark.stroke,
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

        Ok(Self {
            verts: buffers.vertices,
            indices: buffers.indices,
            shader: include_str!("path.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }

    pub fn from_trail_mark(mark: &TrailMark) -> Result<Self, Sg2dWgpuError> {
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

        // Create vertex/index buffer builder
        let mut buffers: VertexBuffers<PathVertex, u16> = VertexBuffers::new();
        let mut buffers_builder = BuffersBuilder::new(
            &mut buffers,
            VertexPositions {
                fill: [0.0, 0.0, 0.0, 0.0],
                stroke: mark.stroke,
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

        Ok(Self {
            verts: buffers.vertices,
            indices: buffers.indices,
            shader: include_str!("path.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }
}

impl BasicMarkShader for PathShader {
    type Vertex = PathVertex;

    fn verts(&self) -> &[Self::Vertex] {
        self.verts.as_slice()
    }

    fn indices(&self) -> &[u16] {
        self.indices.as_slice()
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
}

impl FillVertexConstructor<PathVertex> for VertexPositions {
    fn new_vertex(&mut self, vertex: FillVertex) -> PathVertex {
        PathVertex {
            position: [vertex.position().x, vertex.position().y],
            color: self.fill,
        }
    }
}

impl StrokeVertexConstructor<PathVertex> for VertexPositions {
    fn new_vertex(&mut self, vertex: StrokeVertex) -> PathVertex {
        PathVertex {
            position: [vertex.position().x, vertex.position().y],
            color: self.stroke,
        }
    }
}
