use crate::marks::mark::MarkShader;
use itertools::izip;
use sg2d::marks::arc::ArcMark;
use std::f32::consts::TAU;
use std::mem;
use wgpu::VertexBufferLayout;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ArcVertex {
    pub position: [f32; 2],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
];

impl ArcVertex {
    pub fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<ArcVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ArcInstance {
    pub position: [f32; 2],
    pub start_angle: f32,
    pub end_angle: f32,
    pub outer_radius: f32,
    pub inner_radius: f32,
    pub pad_angle: f32,
    pub corner_radius: f32,
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    pub stroke_width: f32,
}

// First shader index (i.e. the 1 in `1 => Float...`) must be one greater than
// the largest shader index used in VERTEX_ATTRIBUTES above
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 10] = wgpu::vertex_attr_array![
    1 => Float32x2,     // position
    2 => Float32,       // start_angle
    3 => Float32,       // end_angle
    4 => Float32,       // outer_radius
    5 => Float32,       // inner_radius
    6 => Float32,       // pad_angle
    7 => Float32,       // corner_radius
    8 => Float32x4,     // fill
    9 => Float32x4,     // stroke
    10 => Float32,       // stroke_width
];

impl ArcInstance {
    pub fn iter_from_spec(mark: &ArcMark) -> impl Iterator<Item = ArcInstance> + '_ {
        izip!(
            mark.x_iter(),
            mark.y_iter(),
            mark.start_angle_iter(),
            mark.end_angle_iter(),
            mark.outer_radius_iter(),
            mark.inner_radius_iter(),
            mark.pad_angle_iter(),
            mark.corner_radius_iter(),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter(),
        )
        .map(
            |(
                x,
                y,
                start_angle,
                end_angle,
                outer_radius,
                inner_radius,
                pad_angle,
                corner_radius,
                fill,
                stroke,
                stroke_width,
            )| {
                // Normalize start and end angles so that start is in [0, TAU)
                let mut start_angle = *start_angle;
                let mut end_angle = *end_angle;
                if end_angle < start_angle {
                    mem::swap(&mut start_angle, &mut end_angle);
                }
                while start_angle < 0.0 {
                    start_angle += TAU;
                    end_angle += TAU;
                }
                while start_angle >= TAU {
                    start_angle -= TAU;
                    end_angle -= TAU;
                }

                ArcInstance {
                    position: [*x, *y],
                    start_angle,
                    end_angle,
                    // start_angle: *start_angle,
                    // end_angle: *end_angle,
                    outer_radius: outer_radius.max(*inner_radius),
                    inner_radius: inner_radius.min(*outer_radius),
                    pad_angle: *pad_angle,
                    corner_radius: *corner_radius,
                    fill: *fill,
                    stroke: *stroke,
                    stroke_width: *stroke_width,
                }
            },
        )
    }
}

pub struct ArcShader {
    verts: Vec<ArcVertex>,
    indices: Vec<u16>,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl ArcShader {
    pub fn new() -> Self {
        Self {
            verts: vec![
                ArcVertex {
                    position: [-1.0, -1.0],
                },
                ArcVertex {
                    position: [1.0, -1.0],
                },
                ArcVertex {
                    position: [1.0, 1.0],
                },
                ArcVertex {
                    position: [-1.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            shader: include_str!("arc.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        }
    }
}

impl MarkShader for ArcShader {
    type Instance = ArcInstance;
    type Vertex = ArcVertex;

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
            array_stride: std::mem::size_of::<ArcInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        }
    }

    fn vertex_desc(&self) -> VertexBufferLayout<'static> {
        ArcVertex::desc()
    }
}
