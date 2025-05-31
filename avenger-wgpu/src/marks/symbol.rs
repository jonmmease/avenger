use std::hash::{Hash, Hasher};

use crate::error::AvengerWgpuError;
use crate::marks::instanced_mark::{InstancedMarkBatch, InstancedMarkShader};
use avenger_common::canvas::CanvasDimensions;
use avenger_common::types::PathTransform;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use itertools::izip;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillVertex, FillVertexConstructor, StrokeVertex, StrokeVertexConstructor,
};
use lyon::tessellation::geometry_builder::VertexBuffers;
use lyon::tessellation::{FillOptions, FillTessellator, StrokeOptions, StrokeTessellator};
use ordered_float::OrderedFloat;
use wgpu::{Extent3d, VertexBufferLayout};

use super::instanced_mark::InstancedMarkFingerprint;

const FILL_KIND: u32 = 0;
const STROKE_KIND: u32 = 1;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SymbolUniform {
    pub size: [f32; 2],
    pub origin: [f32; 2],
    pub scale: f32,
    _pad: [f32; 5],
}

impl SymbolUniform {
    pub fn new(dimensions: CanvasDimensions, origin: [f32; 2]) -> Self {
        Self {
            size: dimensions.size,
            scale: dimensions.scale,
            origin,
            _pad: [0.0, 0.0, 0.0, 0.0, 0.0],
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
    pub relative_scale: f32,
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
    8 => Float32,       // relative_scale
    9 => Float32,       // angle
    10 => Uint32,       // shape_index
];

impl SymbolInstance {
    pub fn from_spec(
        mark: &SceneSymbolMark,
        max_size: f32,
    ) -> (Vec<SymbolInstance>, Option<image::DynamicImage>, Extent3d) {
        let max_scale = max_size.sqrt();
        let stroke_width = mark.stroke_width.unwrap_or(0.0);
        let mut instances: Vec<SymbolInstance> = Vec::new();
        for (x, y, fill, size, stroke, angle, shape_index) in izip!(
            // Un-adjust x
            mark.x
                .as_iter_owned(mark.len as usize, mark.indices.as_ref()),
            // Un-adjust y
            mark.y
                .as_iter_owned(mark.len as usize, mark.indices.as_ref()),
            mark.fill_iter(),
            mark.size_iter(),
            mark.stroke_iter(),
            mark.angle_iter(),
            mark.shape_index_iter(),
        ) {
            instances.push(SymbolInstance {
                position: [x, y],
                fill_color: fill.color_or_transparent(),
                stroke_color: stroke.color_or_transparent(),
                stroke_width,
                relative_scale: (*size).sqrt() / max_scale,
                angle: *angle,
                shape_index: (*shape_index) as u32,
            });
        }

        (instances, None, Default::default())
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
        mark: &SceneSymbolMark,
        dimensions: CanvasDimensions,
        origin: [f32; 2],
    ) -> Result<Self, AvengerWgpuError> {
        let shapes = &mark.shapes;
        let max_size = mark.max_size();
        let max_scale = max_size.sqrt();
        let mut verts: Vec<SymbolVertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();
        for (shape_index, shape) in shapes.iter().enumerate() {
            let shape_index = shape_index as u32;
            let path = shape.as_path();

            // Scale path to match the size of the largest symbol so tesselation looks nice
            let scaled_path = path
                .as_ref()
                .clone()
                .transformed(&PathTransform::scale(max_scale, max_scale));

            let mut buffers: VertexBuffers<SymbolVertex, u16> = VertexBuffers::new();
            let mut builder = BuffersBuilder::new(&mut buffers, VertexPositions { shape_index });

            // Tesselate fill
            let mut fill_tessellator = FillTessellator::new();
            let fill_options = FillOptions::default().with_tolerance(0.1);
            fill_tessellator.tessellate_path(&scaled_path, &fill_options, &mut builder)?;

            // Tesselate stroke
            if mark.stroke_width.is_some() {
                let mut stroke_tessellator = StrokeTessellator::new();
                let stroke_options = StrokeOptions::default()
                    .with_tolerance(0.1)
                    .with_line_width(1.0);
                stroke_tessellator.tessellate_path(&scaled_path, &stroke_options, &mut builder)?;
            }

            let index_offset = verts.len() as u16;
            verts.extend(buffers.vertices);
            indices.extend(buffers.indices.into_iter().map(|i| i + index_offset));
        }
        let (instances, img, texture_size) = SymbolInstance::from_spec(mark, max_size);
        let batches = vec![InstancedMarkBatch {
            instances_range: 0..instances.len() as u32,
            image: img,
        }];
        Ok(Self {
            verts,
            indices,
            instances,
            uniform: SymbolUniform::new(dimensions, origin),
            batches,
            texture_size,
            shader: include_str!("symbol.wgsl").into(),
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

impl InstancedMarkFingerprint for SceneSymbolMark {
    fn instanced_fingerprint(&self) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();

        self.clip.hash(&mut hasher);
        self.len.hash(&mut hasher);
        self.gradients.hash(&mut hasher);
        self.shapes.hash(&mut hasher);
        self.stroke_width.map(OrderedFloat::from).hash(&mut hasher);
        self.shape_index.hash(&mut hasher);
        self.x.hash(&mut hasher);
        self.y.hash(&mut hasher);
        self.fill.hash(&mut hasher);
        self.size.hash(&mut hasher);
        self.stroke.hash(&mut hasher);
        self.angle.hash(&mut hasher);
        self.indices.hash(&mut hasher);

        
        hasher.finish()
    }
}
