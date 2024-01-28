use crate::canvas::CanvasDimensions;
use crate::marks::gradient::{build_gradients_image, to_color_or_gradient_coord};
use crate::marks::instanced_mark::{InstancedMarkBatch, InstancedMarkShader};
use avenger::marks::group::GroupBounds;
use avenger::marks::rect::RectMark;
use itertools::izip;
use wgpu::{Extent3d, VertexBufferLayout};

pub const GRADIENT_LINEAR: f32 = 0.0;
pub const GRADIENT_RADIAL: f32 = 1.0;

pub const COLORWAY_LENGTH: u32 = 250;
pub const GRADIENT_TEXTURE_WIDTH: u32 = 256;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RectUniform {
    pub size: [f32; 2],
    pub origin: [f32; 2],
    pub group_size: [f32; 2],
    pub scale: f32,
    pub clip: f32,
}

impl RectUniform {
    pub fn new(dimensions: CanvasDimensions, group_bounds: GroupBounds, clip: bool) -> Self {
        Self {
            size: dimensions.size,
            scale: dimensions.scale,
            origin: [group_bounds.x, group_bounds.y],
            group_size: [
                group_bounds.width.unwrap_or(0.0),
                group_bounds.height.unwrap_or(0.0),
            ],
            clip: if clip && group_bounds.width.is_some() && group_bounds.height.is_some() {
                1.0
            } else {
                0.0
            },
        }
    }
}

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
    pub fn from_spec(
        mark: &RectMark,
    ) -> (Vec<RectInstance>, Option<image::DynamicImage>, Extent3d) {
        let mut instances: Vec<RectInstance> = Vec::new();
        let (img, texture_size) = build_gradients_image(&mark.gradients);

        for (x, y, width, height, fill, stroke, stroke_width, corner_radius) in izip!(
            mark.x_iter(),
            mark.y_iter(),
            mark.width_iter(),
            mark.height_iter(),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter(),
            mark.corner_radius_iter(),
        ) {
            instances.push(RectInstance {
                position: [*x, *y],
                width: *width,
                height: *height,
                fill: to_color_or_gradient_coord(fill, texture_size),
                stroke: to_color_or_gradient_coord(stroke, texture_size),
                stroke_width: *stroke_width,
                corner_radius: *corner_radius,
            })
        }
        (instances, img, texture_size)
    }
}

pub struct RectShader {
    verts: Vec<RectVertex>,
    indices: Vec<u16>,
    instances: Vec<RectInstance>,
    uniform: RectUniform,
    batches: Vec<InstancedMarkBatch>,
    texture_size: Extent3d,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl RectShader {
    pub fn from_rect_mark(
        mark: &RectMark,
        dimensions: CanvasDimensions,
        group_bounds: GroupBounds,
    ) -> Self {
        let (instances, img, texture_size) = RectInstance::from_spec(mark);

        let batches = vec![InstancedMarkBatch {
            instances_range: 0..instances.len() as u32,
            image: img,
        }];

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
            instances,
            batches,
            texture_size,
            uniform: RectUniform::new(dimensions, group_bounds, mark.clip),
            shader: format!(
                "{}\n{}",
                include_str!("rect.wgsl"),
                include_str!("gradient.wgsl")
            ),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        }
    }
}

impl InstancedMarkShader for RectShader {
    type Instance = RectInstance;
    type Vertex = RectVertex;
    type Uniform = RectUniform;

    fn verts(&self) -> &[Self::Vertex] {
        self.verts.as_slice()
    }

    fn indices(&self) -> &[u16] {
        self.indices.as_slice()
    }

    fn instances(&self) -> &[Self::Instance] {
        self.instances.as_slice()
    }

    fn uniform(&self) -> Self::Uniform {
        self.uniform
    }

    fn batches(&self) -> &[InstancedMarkBatch] {
        self.batches.as_slice()
    }

    fn texture_size(&self) -> Extent3d {
        self.texture_size
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
