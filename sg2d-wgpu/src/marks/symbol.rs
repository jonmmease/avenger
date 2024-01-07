use crate::error::Sg2dWgpuError;
use crate::marks::mark::MarkShader;
use itertools::izip;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillVertex, FillVertexConstructor, StrokeVertex, StrokeVertexConstructor,
};
use lyon::tessellation::geometry_builder::VertexBuffers;
use lyon::tessellation::{FillOptions, FillTessellator, StrokeOptions, StrokeTessellator};
use sg2d::marks::symbol::{SymbolMark, SymbolShape};
use wgpu::VertexBufferLayout;

const FILL_KIND: u32 = 0;
const STROKE_KIND: u32 = 1;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SymbolVertex {
    pub position: [f32; 2],
    pub normal: [f32; 2],
    pub kind: u32,
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
    1 => Float32x2,     // normal
    2 => Uint32,        // kind
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
}

// First shader index (i.e. the 1 in `1 => Float...`) must be one greater than
// the largest shader index used in VERTEX_ATTRIBUTES above
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    3 => Float32x2,     // position
    4 => Float32x4,     // fill_color
    5 => Float32x4,     // stroke_color
    6 => Float32,       // stroke_width
    7 => Float32,       // size
    8 => Float32,       // angle
];

impl SymbolInstance {
    pub fn iter_from_spec(mark: &SymbolMark) -> impl Iterator<Item = SymbolInstance> + '_ {
        let stroke_width = mark.stroke_width.unwrap_or(0.0);
        izip!(
            mark.x_iter(),
            mark.y_iter(),
            mark.fill_iter(),
            mark.size_iter(),
            mark.stroke_iter(),
            mark.angle_iter(),
        )
        .map(move |(x, y, fill, size, stroke, angle)| SymbolInstance {
            position: [*x, *y],
            fill_color: *fill,
            stroke_color: *stroke,
            stroke_width,
            size: *size,
            angle: *angle,
        })
    }
}

pub struct SymbolShader {
    verts: Vec<SymbolVertex>,
    indices: Vec<u16>,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl SymbolShader {
    pub fn try_new(
        shape: SymbolShape,
        has_fill: bool,
        has_stroke: bool,
    ) -> Result<Self, Sg2dWgpuError> {
        Ok(match shape {
            SymbolShape::Circle => {
                let r = if has_stroke { 0.9 } else { 0.6 };
                let normal: [f32; 2] = [0.0, 0.0];
                let kind = FILL_KIND;
                Self {
                    verts: vec![
                        SymbolVertex {
                            position: [r, -r],
                            normal,
                            kind,
                        },
                        SymbolVertex {
                            position: [r, r],
                            normal,
                            kind,
                        },
                        SymbolVertex {
                            position: [-r, r],
                            normal,
                            kind,
                        },
                        SymbolVertex {
                            position: [-r, -r],
                            normal,
                            kind,
                        },
                    ],
                    indices: vec![0, 1, 2, 0, 2, 3],
                    shader: include_str!("circle.wgsl").to_string(),
                    vertex_entry_point: "vs_main".to_string(),
                    fragment_entry_point: "fs_main".to_string(),
                }
            }
            SymbolShape::Path(ref path) => {
                let mut buffers: VertexBuffers<SymbolVertex, u16> = VertexBuffers::new();
                let mut builder = BuffersBuilder::new(&mut buffers, VertexPositions);

                // Tesselate fill
                if has_fill {
                    let mut fill_tessellator = FillTessellator::new();
                    let fill_options = FillOptions::default().with_tolerance(0.01);
                    fill_tessellator.tessellate_path(path, &fill_options, &mut builder)?;
                }

                // Tesselate stroke
                if has_stroke {
                    let mut stroke_tessellator = StrokeTessellator::new();
                    let stroke_options = StrokeOptions::default().with_line_width(0.1);
                    stroke_tessellator.tessellate_path(path, &stroke_options, &mut builder)?;
                }

                Self {
                    verts: buffers.vertices,
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
    type Vertex = SymbolVertex;

    fn verts(&self) -> &[Self::Vertex] {
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
            attributes: &INSTANCE_ATTRIBUTES,
        }
    }

    fn vertex_desc(&self) -> VertexBufferLayout<'static> {
        SymbolVertex::desc()
    }
}

pub struct VertexPositions;

impl FillVertexConstructor<SymbolVertex> for VertexPositions {
    fn new_vertex(&mut self, vertex: FillVertex) -> SymbolVertex {
        // - y-coordinate is negated to flip vertically from SVG coordinates (top-left)
        // to canvas coordinates (bottom-left).
        SymbolVertex {
            position: [vertex.position().x, -vertex.position().y],
            normal: [0.0, 0.0],
            kind: FILL_KIND,
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
        }
    }
}
