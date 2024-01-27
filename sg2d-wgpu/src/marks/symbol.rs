use crate::canvas::CanvasDimensions;
use crate::error::Sg2dWgpuError;
use crate::marks::gradient::to_color_or_gradient_coord;
use crate::marks::instanced_mark::{InstancedMarkBatch, InstancedMarkShader};
use crate::marks::rect::{build_gradients_image, GRADIENT_TEXTURE_HEIGHT, GRADIENT_TEXTURE_WIDTH};
use itertools::izip;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillVertex, FillVertexConstructor, StrokeVertex, StrokeVertexConstructor,
};
use lyon::tessellation::geometry_builder::VertexBuffers;
use lyon::tessellation::{FillOptions, FillTessellator, StrokeOptions, StrokeTessellator};
use sg2d::marks::symbol::{SymbolMark, SymbolShape};
use wgpu::{Extent3d, VertexBufferLayout};

const FILL_KIND: u32 = 0;
const STROKE_KIND: u32 = 1;
const CIRCLE_KIND: u32 = 2;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SymbolUniform {
    pub size: [f32; 2],
    pub scale: f32,
    _pad: [f32; 1], // Pad to 16 bytes
}

impl SymbolUniform {
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
pub struct SymbolVertex {
    pub position: [f32; 2],
    pub normal: [f32; 2],
    pub kind: u32,
    pub shape_index: u32,
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
    1 => Float32x2,     // normal
    2 => Uint32,        // kind
    3 => Uint32,        // shape_index
];

impl SymbolVertex {
    pub fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<SymbolVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SymbolInstance {
    pub position: [f32; 2],
    pub fill_color: [f32; 4],
    pub stroke_color: [f32; 4],
    pub stroke_width: f32,
    pub size: f32,
    pub angle: f32,
    pub shape_index: u32,
}

// First shader index (i.e. the 1 in `1 => Float...`) must be one greater than
// the largest shader index used in VERTEX_ATTRIBUTES above
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
    4 => Float32x2,     // position
    5 => Float32x4,     // fill_color
    6 => Float32x4,     // stroke_color
    7 => Float32,       // stroke_width
    8 => Float32,       // size
    9 => Float32,       // angle
    10 => Uint32,       // shape_index
];

impl SymbolInstance {
    pub fn from_spec(mark: &SymbolMark) -> (Vec<SymbolInstance>, image::RgbaImage) {
        let stroke_width = mark.stroke_width.unwrap_or(0.0);
        let mut instances: Vec<SymbolInstance> = Vec::new();
        let img = build_gradients_image(&mark.gradients);

        for (x, y, fill, size, stroke, angle, shape_index) in izip!(
            mark.x_iter(),
            mark.y_iter(),
            mark.fill_iter(),
            mark.size_iter(),
            mark.stroke_iter(),
            mark.angle_iter(),
            mark.shape_index_iter(),
        ) {
            instances.push(SymbolInstance {
                position: [*x, *y],
                fill_color: to_color_or_gradient_coord(fill),
                stroke_color: to_color_or_gradient_coord(stroke),
                stroke_width,
                size: *size,
                angle: *angle,
                shape_index: (*shape_index) as u32,
            });
        }

        (instances, img)
    }
}

pub struct SymbolShader {
    verts: Vec<SymbolVertex>,
    indices: Vec<u16>,
    instances: Vec<SymbolInstance>,
    uniform: SymbolUniform,
    batches: Vec<InstancedMarkBatch>,
    texture_size: Extent3d,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl SymbolShader {
    pub fn from_symbol_mark(
        mark: &SymbolMark,
        dimensions: CanvasDimensions,
    ) -> Result<Self, Sg2dWgpuError> {
        let shapes = &mark.shapes;
        let has_stroke = mark.stroke_width.is_some();
        let mut verts: Vec<SymbolVertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();
        for (shape_index, shape) in shapes.iter().enumerate() {
            let shape_index = shape_index as u32;
            match shape {
                SymbolShape::Circle => {
                    let r = if has_stroke { 1.0 } else { 0.6 };
                    let normal: [f32; 2] = [0.0, 0.0];
                    let kind = CIRCLE_KIND;
                    let index_offset = verts.len() as u16;

                    verts.extend(vec![
                        SymbolVertex {
                            position: [r, -r],
                            normal,
                            kind,
                            shape_index,
                        },
                        SymbolVertex {
                            position: [r, r],
                            normal,
                            kind,
                            shape_index,
                        },
                        SymbolVertex {
                            position: [-r, r],
                            normal,
                            kind,
                            shape_index,
                        },
                        SymbolVertex {
                            position: [-r, -r],
                            normal,
                            kind,
                            shape_index,
                        },
                    ]);
                    let local_indices = vec![0, 1, 2, 0, 2, 3];
                    indices.extend(local_indices.into_iter().map(|i| i + index_offset));
                }
                SymbolShape::Path(path) => {
                    let mut buffers: VertexBuffers<SymbolVertex, u16> = VertexBuffers::new();
                    let mut builder =
                        BuffersBuilder::new(&mut buffers, VertexPositions { shape_index });

                    // Tesselate fill
                    let mut fill_tessellator = FillTessellator::new();
                    let fill_options = FillOptions::default().with_tolerance(0.01);
                    fill_tessellator.tessellate_path(path, &fill_options, &mut builder)?;

                    // Tesselate stroke
                    if mark.stroke_width.is_some() {
                        let mut stroke_tessellator = StrokeTessellator::new();
                        let stroke_options = StrokeOptions::default()
                            .with_tolerance(0.01)
                            .with_line_width(0.1);
                        stroke_tessellator.tessellate_path(path, &stroke_options, &mut builder)?;
                    }

                    let index_offset = verts.len() as u16;
                    verts.extend(buffers.vertices);
                    indices.extend(buffers.indices.into_iter().map(|i| i + index_offset));
                }
            }
        }
        let (instances, img) = SymbolInstance::from_spec(mark);
        let batches = vec![InstancedMarkBatch {
            instances_range: 0..instances.len() as u32,
            image: image::DynamicImage::ImageRgba8(img),
        }];
        Ok(Self {
            verts,
            indices,
            instances,
            uniform: SymbolUniform::new(dimensions),
            batches,
            texture_size: Extent3d {
                width: GRADIENT_TEXTURE_WIDTH,
                height: GRADIENT_TEXTURE_HEIGHT,
                depth_or_array_layers: 1,
            },
            shader: include_str!("symbol.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }
}

impl InstancedMarkShader for SymbolShader {
    type Instance = SymbolInstance;
    type Vertex = SymbolVertex;
    type Uniform = SymbolUniform;

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
            array_stride: std::mem::size_of::<SymbolInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        }
    }

    fn vertex_desc(&self) -> VertexBufferLayout<'static> {
        SymbolVertex::desc()
    }
}

pub struct VertexPositions {
    shape_index: u32,
}

impl FillVertexConstructor<SymbolVertex> for VertexPositions {
    fn new_vertex(&mut self, vertex: FillVertex) -> SymbolVertex {
        // - y-coordinate is negated to flip vertically from SVG coordinates (top-left)
        // to canvas coordinates (bottom-left).
        SymbolVertex {
            position: [vertex.position().x, -vertex.position().y],
            normal: [0.0, 0.0],
            kind: FILL_KIND,
            shape_index: self.shape_index,
        }
    }
}

impl StrokeVertexConstructor<SymbolVertex> for VertexPositions {
    fn new_vertex(&mut self, vertex: StrokeVertex) -> SymbolVertex {
        // - y-coordinate is negated to flip vertically from SVG coordinates (top-left)
        // to canvas coordinates (bottom-left).
        SymbolVertex {
            position: [vertex.position().x, -vertex.position().y],
            normal: [vertex.normal().x, -vertex.normal().y],
            kind: STROKE_KIND,
            shape_index: self.shape_index,
        }
    }
}
