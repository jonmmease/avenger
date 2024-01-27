use crate::canvas::CanvasDimensions;
use crate::marks::gradient::to_color_or_gradient_coord;
use crate::marks::instanced_mark::{InstancedMarkBatch, InstancedMarkShader};
use colorgrad::Color;
use image::Rgba;
use itertools::izip;
use sg2d::marks::rect::RectMark;
use sg2d::marks::value::Gradient;
use wgpu::{Extent3d, VertexBufferLayout};

pub const GRADIENT_LINEAR: f32 = 1.0;
pub const GRADIENT_RADIAL: f32 = 2.0;

pub const COLORWAY_LENGTH: u32 = 250;
pub const GRADIENT_TEXTURE_WIDTH: u32 = 256;
pub const GRADIENT_TEXTURE_HEIGHT: u32 = 256;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RectUniform {
    pub size: [f32; 2],
    pub scale: f32,
    _pad: [f32; 1], // Pad to 16 bytes
}

impl RectUniform {
    pub fn new(dimensions: CanvasDimensions) -> Self {
        Self {
            size: dimensions.size,
            scale: dimensions.scale,
            _pad: [0.0],
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

pub fn build_gradients_image(gradients: &[Gradient]) -> image::RgbaImage {
    // Write gradients
    assert!(
        gradients.len() < (GRADIENT_TEXTURE_HEIGHT / 2) as usize,
        "Exceeded max number of unique gradients"
    );

    let mut img = image::RgbaImage::new(GRADIENT_TEXTURE_WIDTH, GRADIENT_TEXTURE_HEIGHT);
    for (pos, grad) in gradients.iter().enumerate() {
        let row0 = (pos * 2) as u32;

        // Build gradient colorway using colorgrad
        let s = grad.stops();
        let mut binding = colorgrad::CustomGradient::new();
        let offsets = s.iter().map(|stop| stop.offset as f64).collect::<Vec<_>>();
        let colors = s
            .iter()
            .map(|stop| {
                Color::new(
                    stop.color[0] as f64,
                    stop.color[1] as f64,
                    stop.color[2] as f64,
                    stop.color[3] as f64,
                )
            })
            .collect::<Vec<_>>();

        let builder = binding.domain(offsets.as_slice()).colors(colors.as_slice());
        let b = builder.build().unwrap();

        // Fill leading pixels with start color so that linear interpolation doesn't pick
        // up the empty pixels between control pixels and gradient pixels
        let start_color = Rgba::from(b.at(0.0).to_rgba8());
        let col_offset = GRADIENT_TEXTURE_WIDTH - COLORWAY_LENGTH;
        for i in 0..col_offset {
            img.put_pixel(i, row0, start_color);
            img.put_pixel(i, row0 + 1, start_color);
        }

        // Store 250-bin colorway in pixels 6 through 255
        for i in 0..COLORWAY_LENGTH {
            let p = (i as f64) / (COLORWAY_LENGTH as f64 - 1.0);
            let c = b.at(p).to_rgba8();

            // Write color to row0 and row0 + 1
            img.put_pixel(i + col_offset, row0, Rgba::from(c));
            img.put_pixel(i + col_offset, row0 + 1, Rgba::from(c));
        }

        // We encode the gradient control points in the first two or three pixels of the texture
        match grad {
            Gradient::LinearGradient(grad) => {
                // Write gradient type to column 0
                let control_color0 = Rgba::from([(GRADIENT_LINEAR * 255.0) as u8, 0, 0, 0]);
                img.put_pixel(0, row0, control_color0);
                img.put_pixel(0, row0 + 1, control_color0);

                // Write x/y control points to column 1
                let control_color1 = Rgba::from([
                    (grad.x0 * 255.0) as u8,
                    (grad.y0 * 255.0) as u8,
                    (grad.x1 * 255.0) as u8,
                    (grad.y1 * 255.0) as u8,
                ]);
                img.put_pixel(1, row0, control_color1);
                img.put_pixel(1, row0 + 1, control_color1);
            }
            Gradient::RadialGradient(grad) => {
                // Write gradient type to column 0
                let control_color0 = Rgba::from([(GRADIENT_RADIAL * 255.0) as u8, 0, 0, 0]);
                img.put_pixel(0, row0, control_color0);
                img.put_pixel(0, row0 + 1, control_color0);

                // Write x/y control points to column 1
                let control_color1 = Rgba::from([
                    (grad.x0 * 255.0) as u8,
                    (grad.y0 * 255.0) as u8,
                    (grad.x1 * 255.0) as u8,
                    (grad.y1 * 255.0) as u8,
                ]);
                img.put_pixel(1, row0, control_color1);
                img.put_pixel(1, row0 + 1, control_color1);

                // Write radius control points to column 2
                let control_color2 =
                    Rgba::from([(grad.r0 * 255.0) as u8, (grad.r1 * 255.0) as u8, 0, 0]);
                img.put_pixel(2, row0, control_color2);
                img.put_pixel(2, row0 + 1, control_color2);
            }
        };
    }
    img
}

impl RectInstance {
    pub fn from_spec(mark: &RectMark) -> (Vec<RectInstance>, image::RgbaImage) {
        let mut instances: Vec<RectInstance> = Vec::new();
        let img = build_gradients_image(&mark.gradients);

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
                fill: to_color_or_gradient_coord(&fill),
                stroke: to_color_or_gradient_coord(&stroke),
                stroke_width: *stroke_width,
                corner_radius: *corner_radius,
            })
        }
        (instances, img)
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
    pub fn from_rect_mark(mark: &RectMark, dimensions: CanvasDimensions) -> Self {
        let (instances, img) = RectInstance::from_spec(mark);

        let batches = vec![InstancedMarkBatch {
            instances_range: 0..instances.len() as u32,
            image: image::DynamicImage::ImageRgba8(img),
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
            texture_size: Extent3d {
                width: GRADIENT_TEXTURE_WIDTH,
                height: GRADIENT_TEXTURE_HEIGHT,
                depth_or_array_layers: 1,
            },
            uniform: RectUniform::new(dimensions),
            shader: include_str!("rect.wgsl").to_string(),
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
