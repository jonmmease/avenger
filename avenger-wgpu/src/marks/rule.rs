use crate::canvas::CanvasDimensions;
use crate::marks::gradient::{build_gradients_image, to_color_or_gradient_coord};
use crate::marks::instanced_mark::{InstancedMarkBatch, InstancedMarkShader};
use avenger::marks::group::GroupBounds;
use avenger::marks::rule::RuleMark;
use avenger::marks::value::StrokeCap;
use image::DynamicImage;
use itertools::izip;
use wgpu::{Extent3d, VertexBufferLayout};

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RuleUniform {
    pub size: [f32; 2],
    pub origin: [f32; 2],
    pub group_size: [f32; 2],
    pub scale: f32,
    pub clip: f32,
}

impl RuleUniform {
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
pub struct RuleVertex {
    pub position: [f32; 2],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
];

impl RuleVertex {
    pub fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<RuleVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }
    }
}

const STROKE_CAP_BUTT: u32 = 0;
const STROKE_CAP_SQUARE: u32 = 1;
const STROKE_CAP_ROUND: u32 = 2;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RuleInstance {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub stroke: [f32; 4],
    pub stroke_width: f32,
    pub stroke_cap: u32,
}

const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
    1 => Float32,     // x0
    2 => Float32,     // y0
    3 => Float32,     // x1
    4 => Float32,     // y1
    5 => Float32x4,   // stroke
    6 => Float32,     // stroke_width
    7 => Uint32,      // stroke_cap_type
];

impl RuleInstance {
    pub fn from_spec(mark: &RuleMark) -> (Vec<RuleInstance>, Option<DynamicImage>, Extent3d) {
        let mut instances: Vec<RuleInstance> = Vec::new();
        let (img, texture_size) = build_gradients_image(&mark.gradients);

        if let Some(stroke_dash_iter) = mark.stroke_dash_iter() {
            // Rule has a dash specification, so we create an individual RuleInstance for each dash
            // in each Rule mark item.

            for (stroke_dash, x0, y0, x1, y1, stroke, stroke_width, cap) in izip!(
                stroke_dash_iter,
                mark.x0_iter(),
                mark.y0_iter(),
                mark.x1_iter(),
                mark.y1_iter(),
                mark.stroke_iter(),
                mark.stroke_width_iter(),
                mark.stroke_cap_iter(),
            ) {
                // Next index into stroke_dash array
                let mut dash_idx = 0;

                // Distance along line from (x0,y0) to (x1,y1) where the next dash will start
                let mut start_dash_dist: f32 = 0.0;

                // Length of the line from (x0,y0) to (x1,y1)
                let rule_len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();

                // Coponents of unit vector along (x0,y0) to (x1,y1)
                let xhat = (x1 - x0) / rule_len;
                let yhat = (y1 - y0) / rule_len;

                // Whether the next dash length represents a drawn dash (draw == true)
                // or a gap (draw == false)
                let mut draw = true;

                while start_dash_dist < rule_len {
                    let end_dash_dist = if start_dash_dist + stroke_dash[dash_idx] >= rule_len {
                        // The final dash/gap should be truncated to the end of the rule
                        rule_len
                    } else {
                        // The dash/gap fits entirely in the rule
                        start_dash_dist + stroke_dash[dash_idx]
                    };

                    if draw {
                        let dash_x0 = x0 + xhat * start_dash_dist;
                        let dash_y0 = y0 + yhat * start_dash_dist;
                        let dash_x1 = x0 + xhat * end_dash_dist;
                        let dash_y1 = y0 + yhat * end_dash_dist;

                        instances.push(RuleInstance {
                            x0: dash_x0,
                            y0: dash_y0,
                            x1: dash_x1,
                            y1: dash_y1,
                            stroke: to_color_or_gradient_coord(stroke, texture_size),
                            stroke_width: *stroke_width,
                            stroke_cap: match cap {
                                StrokeCap::Butt => STROKE_CAP_BUTT,
                                StrokeCap::Square => STROKE_CAP_SQUARE,
                                StrokeCap::Round => STROKE_CAP_ROUND,
                            },
                        })
                    }

                    // update start dist for next dash/gap
                    start_dash_dist = end_dash_dist;

                    // increment index and cycle back to start of start of dash array
                    dash_idx = (dash_idx + 1) % stroke_dash.len();

                    // Alternate between drawn dash and gap
                    draw = !draw;
                }
            }
        } else {
            // Rule has no dash specification, so we create one RuleInstance per Rule mark item
            for (x0, y0, x1, y1, stroke, stroke_width, cap) in izip!(
                mark.x0_iter(),
                mark.y0_iter(),
                mark.x1_iter(),
                mark.y1_iter(),
                mark.stroke_iter(),
                mark.stroke_width_iter(),
                mark.stroke_cap_iter(),
            ) {
                instances.push(RuleInstance {
                    x0: *x0,
                    y0: *y0,
                    x1: *x1,
                    y1: *y1,
                    stroke: to_color_or_gradient_coord(stroke, texture_size),
                    stroke_width: *stroke_width,
                    stroke_cap: match cap {
                        StrokeCap::Butt => STROKE_CAP_BUTT,
                        StrokeCap::Square => STROKE_CAP_SQUARE,
                        StrokeCap::Round => STROKE_CAP_ROUND,
                    },
                })
            }
        }
        (instances, img, texture_size)
    }
}

pub struct RuleShader {
    verts: Vec<RuleVertex>,
    indices: Vec<u16>,
    instances: Vec<RuleInstance>,
    uniform: RuleUniform,
    batches: Vec<InstancedMarkBatch>,
    texture_size: Extent3d,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
}

impl RuleShader {
    pub fn from_rule_mark(
        mark: &RuleMark,
        dimensions: CanvasDimensions,
        group_bounds: GroupBounds,
    ) -> Self {
        let (instances, gradient_image, texture_size) = RuleInstance::from_spec(mark);
        let batches = vec![InstancedMarkBatch {
            instances_range: 0..instances.len() as u32,
            image: gradient_image,
        }];
        Self {
            verts: vec![
                RuleVertex {
                    position: [-0.5, 0.5],
                },
                RuleVertex {
                    position: [-0.5, -0.5],
                },
                RuleVertex {
                    position: [0.5, -0.5],
                },
                RuleVertex {
                    position: [0.5, 0.5],
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            instances,
            uniform: RuleUniform::new(dimensions, group_bounds, mark.clip),
            batches,
            texture_size,
            shader: format!(
                "{}\n{}",
                include_str!("rule.wgsl"),
                include_str!("gradient.wgsl")
            ),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        }
    }
}

impl InstancedMarkShader for RuleShader {
    type Instance = RuleInstance;
    type Vertex = RuleVertex;
    type Uniform = RuleUniform;

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
            array_stride: std::mem::size_of::<RuleInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        }
    }

    fn vertex_desc(&self) -> VertexBufferLayout<'static> {
        RuleVertex::desc()
    }
}
