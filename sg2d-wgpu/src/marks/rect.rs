use crate::marks::instanced_mark::InstancedMarkShader;
use itertools::izip;
use sg2d::marks::rect::RectMark;
use wgpu::VertexBufferLayout;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RectVertex {
    pub position: [f32; 2],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
];

impl RectVertex {
    pub fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<RectVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RectInstance {
    pub position: [f32; 2],
    pub fill: [f32; 4],
    pub width: f32,
    pub height: f32,
    pub stroke: [f32; 4],
    pub stroke_width: f32,
    pub corner_radius: f32,
}

const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
    1 => Float32x2,     // position
    2 => Float32x4,     // color
    3 => Float32,       // width
    4 => Float32,       // height
    5 => Float32x4,     // stroke
    6 => Float32,       // stroke_width
    7 => Float32,       // corner_radius
];

impl RectInstance {
    pub fn iter_from_spec(mark: &RectMark) -> impl Iterator<Item = RectInstance> + '_ {
        izip!(
            mark.x_iter(),
            mark.y_iter(),
            mark.width_iter(),
            mark.height_iter(),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter(),
            mark.corner_radius_iter(),
        )
        .map(
            |(x, y, width, height, fill, stroke, stroke_width, corner_radius)| RectInstance {
                position: [*x, *y],
                width: *width,
                height: *height,
                fill: *fill,
                stroke: *stroke,
                stroke_width: *stroke_width,
                corner_radius: *corner_radius,
            },
        )
    }
}

pub struct RectShader {
    verts: Vec<RectVertex>,
    indices: Vec<u16>,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl Default for RectShader {
    fn default() -> Self {
        Self::new()
    }
}

impl RectShader {
    pub fn new() -> Self {
        Self {
            verts: vec![
                RectVertex {
                    position: [0.0, 0.0],
                },
                RectVertex {
                    position: [1.0, 0.0],
                },
                RectVertex {
                    position: [1.0, 1.0],
                },
                RectVertex {
                    position: [0.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            shader: include_str!("rect.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        }
    }
}

impl InstancedMarkShader for RectShader {
    type Instance = RectInstance;
    type Vertex = RectVertex;

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

    fn instance_desc(&self) -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<RectInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        }
    }

    fn vertex_desc(&self) -> VertexBufferLayout<'static> {
        RectVertex::desc()
    }
}
