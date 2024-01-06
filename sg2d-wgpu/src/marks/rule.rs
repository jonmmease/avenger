use crate::marks::mark::MarkShader;
use crate::vertex::Vertex;
use itertools::izip;
use sg2d::marks::rule::RuleMark;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RuleInstance {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub stroke: [f32; 3],
    pub stroke_width: f32,
}

const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    1 => Float32,     // x0
    2 => Float32,     // y0
    3 => Float32,     // x1
    4 => Float32,     // y1
    5 => Float32x3,   // stroke
    6 => Float32,     // stroke_width
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
        )
        .map(|(x0, y0, x1, y1, stroke, stroke_width)| RuleInstance {
            x0: *x0,
            y0: *y0,
            x1: *x1,
            y1: *y1,
            stroke: *stroke,
            stroke_width: *stroke_width,
        })
    }
}

pub struct RuleShader {
    verts: Vec<Vertex>,
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
                Vertex {
                    position: [-0.5, 0.5],
                },
                Vertex {
                    position: [-0.5, -0.5],
                },
                Vertex {
                    position: [0.5, -0.5],
                },
                Vertex {
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

    fn verts(&self) -> &[Vertex] {
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
}
