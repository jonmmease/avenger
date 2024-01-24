use crate::canvas::CanvasDimensions;
use crate::marks::texture_instanced_mark::{InstancedTextureMarkBatch, TextureInstancedMarkShader};
use itertools::izip;
use sg2d::marks::rect::RectMark;
use sg2d::marks::value::{ColorOrGradient, Gradient, GradientStop};
use wgpu::{Extent3d, VertexBufferLayout};

const GRADIENT_NONE: f32 = 0.0;
const GRADIENT_LINEAR: f32 = 1.0;
const GRADIENT_RADIAL: f32 = 2.0;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GradientRectUniform {
    pub size: [f32; 2],
    pub scale: f32,
    pub gradient_type: f32,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub r0: f32,
    pub r1: f32,
    _pad: [f32; 2], // Pad to 16 bytes
}

impl GradientRectUniform {
    pub fn new(dimensions: CanvasDimensions, gradient: Option<Gradient>) -> Self {
        match gradient {
            None => Self {
                size: dimensions.size,
                scale: dimensions.scale,
                gradient_type: GRADIENT_NONE,
                x0: 0.0,
                y0: 0.0,
                x1: 0.0,
                y1: 0.0,
                r0: 0.0,
                r1: 0.0,
                _pad: [0.0, 0.0],
            },
            Some(Gradient::LinearGradient(gradient)) => Self {
                size: dimensions.size,
                scale: dimensions.scale,
                gradient_type: GRADIENT_LINEAR,
                x0: gradient.x0,
                y0: gradient.y0,
                x1: gradient.x1,
                y1: gradient.y1,
                r0: 0.0,
                r1: 0.0,
                _pad: [0.0, 0.0],
            },
            Some(Gradient::RadialGradient(gradient)) => Self {
                size: dimensions.size,
                scale: dimensions.scale,
                gradient_type: GRADIENT_RADIAL,
                x0: gradient.x0,
                y0: gradient.y0,
                x1: gradient.x1,
                y1: gradient.y1,
                r0: gradient.r0,
                r1: gradient.r1,
                _pad: [0.0, 0.0],
            },
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GradientRectVertex {
    pub position: [f32; 2],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
];

impl GradientRectVertex {
    pub fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<GradientRectVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GradientRectInstance {
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

impl GradientRectInstance {
    pub fn from_spec(mark: &RectMark) -> (Vec<GradientRectInstance>, Option<Gradient>) {
        let mut instances: Vec<GradientRectInstance> = Vec::new();
        let mut gradient: Option<Gradient> = None;
        let mut stops: Vec<Vec<GradientStop>> = Vec::new();

        let mut compute_color = |color_or_gradient: &ColorOrGradient| -> [f32; 4] {
            match color_or_gradient {
                ColorOrGradient::Color(c) => *c,
                ColorOrGradient::Gradient(grad) => {
                    gradient = Some(grad.clone());
                    let s = grad.stops();
                    let pos = if let Some(pos) = stops.iter().position(|s| s.as_slice() == s) {
                        // Already have stops, store index
                        pos
                    } else {
                        // Add stops
                        let pos = stops.len();
                        stops.push(Vec::from(s));
                        pos
                    };
                    // Each gradient stops colorscale is written to two rows, starting an texture
                    // coordinate 0. This results in 128 gradients stored in 256x256 texture. To
                    // avoid interpolation artifacts, we compute the texture coordinate as the
                    // position between the two rows
                    let grad_coord = (pos as f32 / 128.0) + 1.0 / 512.0;
                    [-grad_coord, 0.0, 0.0, 0.0]
                }
            }
        };

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
            let fill = compute_color(fill);
            let stroke = compute_color(stroke);
            instances.push(GradientRectInstance {
                position: [*x, *y],
                width: *width,
                height: *height,
                fill: fill,
                stroke: stroke,
                stroke_width: *stroke_width,
                corner_radius: *corner_radius,
            })
        }
        (instances, gradient)
    }
}

pub struct GradientRectShader {
    verts: Vec<GradientRectVertex>,
    indices: Vec<u16>,
    instances: Vec<GradientRectInstance>,
    uniform: GradientRectUniform,
    batches: Vec<InstancedTextureMarkBatch>,
    texture_size: Extent3d,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl GradientRectShader {
    pub fn from_rect_mark(mark: &RectMark, dimensions: CanvasDimensions) -> Self {
        let (instances, gradient) = GradientRectInstance::from_spec(mark);

        let batches = vec![InstancedTextureMarkBatch {
            instances_range: 0..instances.len() as u32,
            image: image::DynamicImage::ImageRgba8(image::RgbaImage::new(256, 256)),
        }];

        Self {
            verts: vec![
                GradientRectVertex {
                    position: [0.0, 0.0],
                },
                GradientRectVertex {
                    position: [1.0, 0.0],
                },
                GradientRectVertex {
                    position: [1.0, 1.0],
                },
                GradientRectVertex {
                    position: [0.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            instances,
            batches,
            texture_size: Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            uniform: GradientRectUniform::new(dimensions, gradient),
            shader: include_str!("gradient_rect.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        }
    }
}

impl TextureInstancedMarkShader for GradientRectShader {
    type Instance = GradientRectInstance;
    type Vertex = GradientRectVertex;
    type Uniform = GradientRectUniform;

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

    fn batches(&self) -> &[InstancedTextureMarkBatch] {
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
            array_stride: std::mem::size_of::<GradientRectInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        }
    }

    fn vertex_desc(&self) -> VertexBufferLayout<'static> {
        GradientRectVertex::desc()
    }
}
