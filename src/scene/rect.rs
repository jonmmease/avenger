use crate::specs::mark::MarkContainerSpec;
use crate::specs::rect::RectItemSpec;

#[derive(Debug, Clone)]
pub struct RectMark {
    pub instances: Vec<RectInstance>,
    pub clip: bool,
}

impl RectMark {
    pub fn from_spec(spec: &MarkContainerSpec<RectItemSpec>) -> Self {
        let instances = spec.items.iter().map(|item| {
            RectInstance {
                position: [item.x, item.y],
                color: [0.5, 0.5, 0.5],
                width: item.width.unwrap(),
                height: item.height.unwrap(),
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
pub struct RectInstance {
    pub position: [f32; 2],
    pub color: [f32; 3],
    pub width: f32,
    pub height: f32,
}

impl RectInstance {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<RectInstance>() as wgpu::BufferAddress,
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
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 6]>() as wgpu::BufferAddress,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32,
                },
            ]
        }
    }

    pub fn from_spec(item_spec: &RectItemSpec) -> Self {
        // TODO: color, x2, y2
        Self {
            position: [item_spec.x, item_spec.y],
            color: [0.5, 0.5, 0.5],
            width: item_spec.width.unwrap(),
            height: item_spec.height.unwrap(),
        }
    }

    pub fn from_specs(item_specs: &[RectItemSpec]) -> Vec<Self> {
        item_specs.iter().map(Self::from_spec).collect()
    }
}