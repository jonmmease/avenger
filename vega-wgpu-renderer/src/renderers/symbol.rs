use crate::renderers::mark::MarkShader;
use crate::renderers::vertex::Vertex;
use crate::scene::symbol::SymbolInstance;
use crate::specs::symbol::SymbolShape;

pub struct SymbolShader {
    verts: Vec<Vertex>,
    indices: Vec<u16>,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl SymbolShader {
    pub fn new(shape: SymbolShape) -> Self {
        let tan30: f32 = (30.0 * std::f32::consts::PI / 180.0).tan();
        let sqrt3: f32 = 3.0f32.sqrt();

        match shape {
            SymbolShape::Circle => {
                let r = 0.5;
                Self {
                    verts: vec![
                        Vertex {
                            position: [r, -r, 0.0],
                        },
                        Vertex {
                            position: [r, r, 0.0],
                        },
                        Vertex {
                            position: [-r, r, 0.0],
                        },
                        Vertex {
                            position: [-r, -r, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 2, 0, 2, 3],
                    shader: include_str!("circle.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::Square => {
                let r = 0.5;
                Self {
                    verts: vec![
                        Vertex {
                            position: [r, -r, 0.0],
                        },
                        Vertex {
                            position: [r, r, 0.0],
                        },
                        Vertex {
                            position: [-r, r, 0.0],
                        },
                        Vertex {
                            position: [-r, -r, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 2, 0, 2, 3],
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::Cross => {
                let r = 0.5;
                let s = r / 2.5;
                Self {
                    verts: vec![
                        Vertex {
                            position: [-s, r, 0.0],
                        },
                        Vertex {
                            position: [-s, s, 0.0],
                        },
                        Vertex {
                            position: [-r, s, 0.0],
                        },
                        Vertex {
                            position: [-r, -s, 0.0],
                        },
                        Vertex {
                            position: [-s, -s, 0.0],
                        },
                        Vertex {
                            position: [-s, -r, 0.0],
                        },
                        Vertex {
                            position: [s, -r, 0.0],
                        },
                        Vertex {
                            position: [s, -s, 0.0],
                        },
                        Vertex {
                            position: [r, -s, 0.0],
                        },
                        Vertex {
                            position: [r, s, 0.0],
                        },
                        Vertex {
                            position: [s, s, 0.0],
                        },
                        Vertex {
                            position: [s, r, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 10, 0, 10, 11, 2, 3, 8, 2, 8, 9, 4, 5, 7, 5, 6, 7],
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::Diamond => {
                let ry: f32 = (1.0 / (2.0 * tan30)).sqrt();
                let r: f32 = ry * tan30;
                Self {
                    verts: vec![
                        Vertex {
                            position: [0.0, -r, 0.0],
                        },
                        Vertex {
                            position: [r, 0.0, 0.0],
                        },
                        Vertex {
                            position: [0.0, r, 0.0],
                        },
                        Vertex {
                            position: [-r, 0.0, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 2, 0, 2, 3],
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::Triangle => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                let o = (h - r * tan30);
                Self {
                    verts: vec![
                        Vertex {
                            position: [0.0, h + o, 0.0],
                        },
                        Vertex {
                            position: [-r, -h + o, 0.0],
                        },
                        Vertex {
                            position: [r, -h + o, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 2],
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::TriangleUp => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                Self {
                    verts: vec![
                        Vertex {
                            position: [0.0, h, 0.0],
                        },
                        Vertex {
                            position: [-r, -h, 0.0],
                        },
                        Vertex {
                            position: [r, -h, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 2],
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::TriangleDown => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                Self {
                    verts: vec![
                        Vertex {
                            position: [0.0, -h, 0.0],
                        },
                        Vertex {
                            position: [r, h, 0.0],
                        },
                        Vertex {
                            position: [-r, h, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 2],
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::TriangleRight => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                Self {
                    verts: vec![
                        Vertex {
                            position: [h, 0.0, 0.0],
                        },
                        Vertex {
                            position: [-h, r, 0.0],
                        },
                        Vertex {
                            position: [-h, -r, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 2],
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::TriangleLeft => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                Self {
                    verts: vec![
                        Vertex {
                            position: [-h, 0.0, 0.0],
                        },
                        Vertex {
                            position: [h, -r, 0.0],
                        },
                        Vertex {
                            position: [h, r, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 2],
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::Arrow => {
                let r = 0.5;
                let s = r / 7.0;
                let t = r / 2.5;
                let v = r / 8.0;

                Self {
                    verts: vec![
                        Vertex {
                            position: [0.0, r, 0.0],
                        },
                        Vertex {
                            position: [-t, v, 0.0],
                        },
                        Vertex {
                            position: [-s, v, 0.0],
                        },
                        Vertex {
                            position: [-s, -r, 0.0],
                        },
                        Vertex {
                            position: [s, -r, 0.0],
                        },
                        Vertex {
                            position: [s, v, 0.0],
                        },
                        Vertex {
                            position: [t, v, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 6, 2, 3, 4, 2, 4, 5],
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::Wedge => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                let o = (h - r * tan30);
                let b = r / 4.0;
                Self {
                    verts: vec![
                        Vertex {
                            position: [0.0, h + o, 0.0],
                        },
                        Vertex {
                            position: [-b, -h + o, 0.0],
                        },
                        Vertex {
                            position: [b, -h + o, 0.0],
                        },
                    ],
                    indices: vec![0, 1, 2],
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
        }
    }
}

impl MarkShader for SymbolShader {
    type Instance = SymbolInstance;

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
            array_stride: std::mem::size_of::<SymbolInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 5]>() as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32,
                },
            ],
        }
    }
}
