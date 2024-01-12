use crate::marks::mark::MarkShader;
use itertools::izip;
use sg2d::marks::rule::RuleMark;
use sg2d::value::StrokeCap;
use wgpu::VertexBufferLayout;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RuleVertex {
    pub position: [f32; 2],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
];

impl RuleVertex {
    pub fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<RuleVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }
    }
}

const STROKE_CAP_BUTT: u32 = 0;
const STROKE_CAP_SQUARE: u32 = 1;
const STROKE_CAP_ROUND: u32 = 2;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RuleInstance {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub stroke: [f32; 4],
    pub stroke_width: f32,
    pub stroke_cap: u32,
}

const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
    1 => Float32,     // x0
    2 => Float32,     // y0
    3 => Float32,     // x1
    4 => Float32,     // y1
    5 => Float32x4,   // stroke
    6 => Float32,     // stroke_width
    7 => Uint32,      // stroke_cap_type
];

impl RuleInstance {
    pub fn iter_from_spec(mark: &RuleMark) -> impl Iterator<Item = RuleInstance> + '_ {
        izip!(
            mark.x0_iter(),
            mark.y0_iter(),
            mark.x1_iter(),
            mark.y1_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter(),
            mark.stroke_cap_iter(),
        )
        .map(|(x0, y0, x1, y1, stroke, stroke_width, cap)| RuleInstance {
            x0: *x0,
            y0: *y0,
            x1: *x1,
            y1: *y1,
            stroke: *stroke,
            stroke_width: *stroke_width,
            stroke_cap: match cap {
                StrokeCap::Butt => STROKE_CAP_BUTT,
                StrokeCap::Square => STROKE_CAP_SQUARE,
                StrokeCap::Round => STROKE_CAP_ROUND,
            },
        })
    }
}

pub struct RuleShader {
    verts: Vec<RuleVertex>,
    indices: Vec<u16>,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl Default for RuleShader {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleShader {
    pub fn new() -> Self {
        Self {
            verts: vec![
                RuleVertex {
                    position: [-0.5, 0.5],
                },
                RuleVertex {
                    position: [-0.5, -0.5],
                },
                RuleVertex {
                    position: [0.5, -0.5],
                },
                RuleVertex {
                    position: [0.5, 0.5],
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            shader: include_str!("rule.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        }
    }
}

impl MarkShader for RuleShader {
    type Instance = RuleInstance;
    type Vertex = RuleVertex;

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
            array_stride: std::mem::size_of::<RuleInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        }
    }

    fn vertex_desc(&self) -> VertexBufferLayout<'static> {
        RuleVertex::desc()
    }
}
