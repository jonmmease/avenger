use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::values::CssColorOrGradient;
use avenger::marks::arc::ArcMark;
use avenger::marks::mark::SceneMark;
use avenger::marks::value::{ColorOrGradient, EncodingValue, Gradient};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaArcItem {
    pub x: f32,
    pub y: f32,
    pub start_angle: Option<f32>,   // default 0.0
    pub end_angle: Option<f32>,     // default 0.0
    pub outer_radius: Option<f32>,  // default 0.0
    pub inner_radius: Option<f32>,  // default 0.0
    pub pad_angle: Option<f32>,     // default 0.0
    pub corner_radius: Option<f32>, // default 0.0
    pub fill: Option<CssColorOrGradient>,
    pub fill_opacity: Option<f32>, // default 1.0
    pub stroke: Option<CssColorOrGradient>,
    pub stroke_width: Option<f32>,   // default 0.0
    pub stroke_opacity: Option<f32>, // default 1.0
    pub opacity: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaArcItem {}

impl VegaMarkContainer<VegaArcItem> {
    pub fn to_scene_graph(&self) -> Result<SceneMark, AvengerVegaError> {
        // Init mark with scalar defaults
        let mut mark = ArcMark {
            clip: self.clip,
            ..Default::default()
        };
        if let Some(name) = &self.name {
            mark.name = name.clone();
        }

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut start_angle = Vec::<f32>::new();
        let mut end_angle = Vec::<f32>::new();
        let mut outer_radius = Vec::<f32>::new();
        let mut inner_radius = Vec::<f32>::new();
        let mut pad_angle = Vec::<f32>::new();
        let mut corner_radius = Vec::<f32>::new();
        let mut fill = Vec::<ColorOrGradient>::new();
        let mut stroke = Vec::<ColorOrGradient>::new();
        let mut stroke_width = Vec::<f32>::new();
        let mut zindex = Vec::<i32>::new();
        let mut gradients = Vec::<Gradient>::new();

        // For each item, append explicit values to corresponding vector
        for item in &self.items {
            x.push(item.x);
            y.push(item.y);

            if let Some(s) = item.start_angle {
                start_angle.push(s);
            }
            if let Some(s) = item.end_angle {
                end_angle.push(s);
            }
            if let Some(s) = item.outer_radius {
                outer_radius.push(s);
            }
            if let Some(s) = item.inner_radius {
                inner_radius.push(s);
            }
            if let Some(s) = item.pad_angle {
                pad_angle.push(s);
            }
            if let Some(s) = item.corner_radius {
                corner_radius.push(s);
            }
            if let Some(v) = &item.fill {
                let opacity = item.fill_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                fill.push(v.to_color_or_grad(opacity, &mut gradients)?);
            }
            if let Some(v) = &item.stroke {
                let opacity = item.stroke_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                stroke.push(v.to_color_or_grad(opacity, &mut gradients)?);
            }
            if let Some(s) = item.stroke_width {
                stroke_width.push(s);
            }
            if let Some(v) = item.zindex {
                zindex.push(v);
            }
        }

        // Override default scalar values with vectors
        let len = self.items.len();
        mark.len = len as u32;

        if x.len() == len {
            mark.x = EncodingValue::Array { values: x };
        }
        if y.len() == len {
            mark.y = EncodingValue::Array { values: y };
        }
        if start_angle.len() == len {
            mark.start_angle = EncodingValue::Array {
                values: start_angle,
            };
        }
        if end_angle.len() == len {
            mark.end_angle = EncodingValue::Array { values: end_angle };
        }
        if outer_radius.len() == len {
            mark.outer_radius = EncodingValue::Array {
                values: outer_radius,
            };
        }
        if inner_radius.len() == len {
            mark.inner_radius = EncodingValue::Array {
                values: inner_radius,
            };
        }
        if pad_angle.len() == len {
            mark.pad_angle = EncodingValue::Array { values: pad_angle };
        }
        if corner_radius.len() == len {
            mark.corner_radius = EncodingValue::Array {
                values: corner_radius,
            };
        }
        if fill.len() == len {
            mark.fill = EncodingValue::Array { values: fill };
        }
        if stroke.len() == len {
            mark.stroke = EncodingValue::Array { values: stroke };
        }
        if stroke_width.len() == len {
            mark.stroke_width = EncodingValue::Array {
                values: stroke_width,
            };
        }
        if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            mark.indices = Some(indices);
        }

        // Add gradients
        mark.gradients = gradients;

        Ok(SceneMark::Arc(mark))
    }
}
