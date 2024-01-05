use crate::error::Sg2dWgpuError;
use crate::marks::mark::MarkShader;
use crate::vertex::Vertex;
use itertools::izip;
use lyon::tessellation::geometry_builder::{simple_builder, VertexBuffers};
use lyon::tessellation::math::Point;
use lyon::tessellation::{FillOptions, FillTessellator};
use sg2d::marks::symbol::{SymbolMark, SymbolShape};
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SymbolInstance {
    pub position: [f32; 2],
    pub color: [f32; 3],
    pub size: f32,
}

impl SymbolInstance {
    pub fn iter_from_spec(mark: &SymbolMark) -> impl Iterator<Item = SymbolInstance> + '_ {
        izip!(
            mark.x_iter(),
            mark.y_iter(),
            mark.fill_iter(),
            mark.size_iter(),
        )
        .map(|(x, y, fill, size)| SymbolInstance {
            position: [*x, *y],
            color: *fill,
            size: *size,
        })
    }
}

pub struct SymbolShader {
    verts: Vec<Vertex>,
    indices: Vec<u16>,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl SymbolShader {
    pub fn try_new(shape: SymbolShape) -> Result<Self, Sg2dWgpuError> {
        Ok(match shape {
            SymbolShape::Circle => {
                let r = 0.6;
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
            SymbolShape::Path(ref path) => {
                let mut buffers: VertexBuffers<Point, u16> = VertexBuffers::new();
                let mut vertex_builder = simple_builder(&mut buffers);
                let mut tessellator = FillTessellator::new();
                let options = FillOptions::default();
                tessellator.tessellate_path(path, &options, &mut vertex_builder)?;

                // - y-coordinate is negated to flip vertically from SVG coordinates (top-left)
                // to canvas coordinates (bottom-left).
                let verts = buffers
                    .vertices
                    .iter()
                    .map(|v| Vertex {
                        position: [v.x, -v.y, 0.0],
                    })
                    .collect::<Vec<_>>();
                Self {
                    verts,
                    indices: buffers.indices,
                    shader: include_str!("polygon_symbol.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
        })
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
