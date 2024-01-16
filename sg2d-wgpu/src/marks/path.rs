use crate::error::Sg2dWgpuError;
use crate::marks::basic_mark::BasicMarkShader;
use itertools::izip;
use lyon::geom::euclid::{Transform2D, Vector2D};
use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, StrokeOptions,
    StrokeTessellator, StrokeVertex, StrokeVertexConstructor, VertexBuffers,
};
use lyon::math::Angle;
use lyon::path::{LineCap, LineJoin};
use sg2d::marks::path::PathMark;
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
        for (path, x, y, scale_x, scale_y, angle, fill, stroke) in izip!(
            mark.path_iter(),
            mark.x_iter(),
            mark.y_iter(),
            mark.scale_x_iter(),
            mark.scale_y_iter(),
            mark.angle_iter(),
            mark.fill_iter(),
            mark.stroke_iter(),
        ) {
            // Apply scale and rotation to path
            let transform = Transform2D::scale(*scale_x, *scale_y)
                .then_rotate(Angle::degrees(*angle))
                .then_translate(Vector2D::new(*x, *y));

            let path = path.clone().transformed(&transform);

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
