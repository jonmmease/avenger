use crate::marks::mark::MarkShader;
use crate::vertex::Vertex;
use itertools::izip;
use sg2d::marks::rect::RectMark;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RectInstance {
    pub position: [f32; 2],
    pub color: [f32; 3],
    pub width: f32,
    pub height: f32,
}

impl RectInstance {
    pub fn iter_from_spec(mark: &RectMark) -> impl Iterator<Item = RectInstance> + '_ {
        izip!(
            mark.x_iter(),
            mark.y_iter(),
            mark.width_iter(),
            mark.height_iter(),
            mark.fill_iter(),
        )
        .map(|(x, y, width, height, fill)| RectInstance {
            position: [*x, *y],
            width: *width,
            height: *height,
            color: *fill,
        })
    }
}

pub struct RectShader {
    verts: Vec<Vertex>,
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
                Vertex {
                    position: [0.0, 0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 1.0, 0.0],
                },
                Vertex {
                    position: [0.0, 1.0, 0.0],
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            shader: include_str!("rect.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        }
    }
}

impl MarkShader for RectShader {
    type Instance = RectInstance;

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
            ],
        }
    }
}