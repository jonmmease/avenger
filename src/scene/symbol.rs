use crate::specs::mark::MarkContainerSpec;
use crate::specs::symbol::SymbolItemSpec;

#[derive(Debug, Clone)]
pub struct SymbolMark {
    pub instances: Vec<SymbolInstance>,
    pub clip: bool,
}

impl SymbolMark {
    pub fn from_spec(spec: &MarkContainerSpec<SymbolItemSpec>) -> Self {
        let instances = spec.items.iter().map(|item| {
            SymbolInstance {
                position: [item.x, item.y],
                color: [0.5, 0.5, 0.5],
            }
        }).collect::<Vec<_>>();

        Self {
            instances,
            clip: spec.clip,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SymbolInstance {
    pub position: [f32; 2],
    pub color: [f32; 3],
}

impl SymbolInstance {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
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
                }
            ]
        }
    }

    pub fn from_spec(item_spec: &SymbolItemSpec) -> Self {
        // TODO: color
        Self {
            position: [item_spec.x, item_spec.y],
            color: [0.5, 0.5, 0.5],
        }
    }

    pub fn from_specs(item_specs: &[SymbolItemSpec]) -> Vec<Self> {
        item_specs.iter().map(Self::from_spec).collect()
    }
}