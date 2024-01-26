use crate::canvas::CanvasDimensions;
use crate::marks::texture_instanced_mark::{InstancedTextureMarkBatch, TextureInstancedMarkShader};
use colorgrad::Color;
use image::Rgba;
use itertools::izip;
use sg2d::marks::rect::RectMark;
use sg2d::marks::value::{ColorOrGradient, Gradient, GradientStop};
use wgpu::{Extent3d, VertexBufferLayout};

const GRADIENT_LINEAR: f32 = 1.0;
const GRADIENT_RADIAL: f32 = 2.0;

const COLORWAY_LENGTH: u32 = 250;
const GRADIENT_TEXTURE_WIDTH: u32 = 256;
const GRADIENT_TEXTURE_HEIGHT: u32 = 256;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GradientRectUniform {
    pub size: [f32; 2],
    pub scale: f32,
    _pad: [f32; 1], // Pad to 16 bytes
}

impl GradientRectUniform {
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
    pub fn from_spec(mark: &RectMark) -> (Vec<GradientRectInstance>, image::RgbaImage) {
        let mut instances: Vec<GradientRectInstance> = Vec::new();
        let mut stops: Vec<Vec<GradientStop>> = Vec::new();
        let mut img = image::RgbaImage::new(GRADIENT_TEXTURE_WIDTH, GRADIENT_TEXTURE_HEIGHT);

        let mut compute_color = |color_or_gradient: &ColorOrGradient| -> [f32; 4] {
            match color_or_gradient {
                ColorOrGradient::Color(c) => *c,
                ColorOrGradient::Gradient(grad) => {
                    // gradient = Some(grad.clone());
                    let s = grad.stops();
                    let pos = if let Some(pos) = stops.iter().position(|stop| stop.as_slice() == s)
                    {
                        // Already have stops, store index
                        pos
                    } else {
                        // Add stops
                        let pos = stops.len();
                        assert!(
                            pos < (GRADIENT_TEXTURE_HEIGHT / 2) as usize,
                            "Exceeded max number of unique gradients"
                        );
                        let row0 = (pos * 2) as u32;

                        // Build gradient colorway using colorgrad
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
                                let control_color0 =
                                    Rgba::from([(GRADIENT_LINEAR * 255.0) as u8, 0, 0, 0]);
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
                                let control_color0 =
                                    Rgba::from([(GRADIENT_RADIAL * 255.0) as u8, 0, 0, 0]);
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
                                let control_color2 = Rgba::from([
                                    (grad.r0 * 255.0) as u8,
                                    (grad.r1 * 255.0) as u8,
                                    0,
                                    0,
                                ]);
                                img.put_pixel(2, row0, control_color2);
                                img.put_pixel(2, row0 + 1, control_color2);
                            }
                        };

                        stops.push(Vec::from(s));
                        pos
                    };

                    // Each gradient colorway is written to two rows, starting at texture
                    // y-coordinate 0. This results in 128 gradients stored in a 256x256 texture. To
                    // avoid interpolation artifacts, we compute the texture coordinate as the
                    // position between the two rows
                    let grad_coord =
                        (pos as f32 / 128.0) + 1.0 / (GRADIENT_TEXTURE_HEIGHT as f32 * 2.0);
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
        (instances, img)
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
        let (instances, img) = GradientRectInstance::from_spec(mark);

        let batches = vec![InstancedTextureMarkBatch {
            instances_range: 0..instances.len() as u32,
            image: image::DynamicImage::ImageRgba8(img),
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
            uniform: GradientRectUniform::new(dimensions),
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
