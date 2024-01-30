use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::values::{CssColorOrGradient, StrokeDashSpec};
use avenger::marks::mark::SceneMark;
use avenger::marks::rule::RuleMark;
use avenger::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaRuleItem {
    pub x: f32,
    pub y: f32,
    pub x2: Option<f32>,
    pub y2: Option<f32>,
    pub stroke: Option<CssColorOrGradient>,
    pub stroke_width: Option<f32>,
    pub stroke_cap: Option<StrokeCap>,
    pub stroke_opacity: Option<f32>,
    pub stroke_dash: Option<StrokeDashSpec>,
    pub opacity: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaRuleItem {}

impl VegaMarkContainer<VegaRuleItem> {
    pub fn to_scene_graph(&self) -> Result<SceneMark, AvengerVegaError> {
        // Init mark with scalar defaults
        let mut mark = RuleMark {
            clip: self.clip,
            ..Default::default()
        };
        if let Some(name) = &self.name {
            mark.name = name.clone();
        }

        // Init vector for each encoding channel
        let mut x0 = Vec::<f32>::new();
        let mut y0 = Vec::<f32>::new();
        let mut x1 = Vec::<f32>::new();
        let mut y1 = Vec::<f32>::new();
        let mut stroke = Vec::<ColorOrGradient>::new();
        let mut stroke_width = Vec::<f32>::new();
        let mut stroke_cap = Vec::<StrokeCap>::new();
        let mut stroke_dash = Vec::<Vec<f32>>::new();
        let mut zindex = Vec::<i32>::new();
        let mut gradients = Vec::<Gradient>::new();

        // For each item, append explicit values to corresponding vector
        for item in &self.items {
            x0.push(item.x);
            y0.push(item.y);
            x1.push(item.x2.unwrap_or(item.x));
            y1.push(item.y2.unwrap_or(item.y));

            if let Some(v) = &item.stroke {
                let opacity = item.stroke_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                stroke.push(v.to_color_or_grad(opacity, &mut gradients)?);
            }

            if let Some(s) = item.stroke_width {
                stroke_width.push(s);
            }

            if let Some(s) = item.stroke_cap {
                stroke_cap.push(s);
            }

            if let Some(dash) = &item.stroke_dash {
                stroke_dash.push(dash.to_array()?.to_vec());
            }

            if let Some(v) = item.zindex {
                zindex.push(v);
            }
        }

        // Override values with vectors
        let len = self.items.len();
        mark.len = len as u32;

        if x0.len() == len {
            mark.x0 = EncodingValue::Array { values: x0 };
        }
        if y0.len() == len {
            mark.y0 = EncodingValue::Array { values: y0 };
        }
        if x1.len() == len {
            mark.x1 = EncodingValue::Array { values: x1 };
        }
        if y1.len() == len {
            mark.y1 = EncodingValue::Array { values: y1 };
        }
        if stroke.len() == len {
            mark.stroke = EncodingValue::Array { values: stroke };
        }
        if stroke_width.len() == len {
            mark.stroke_width = EncodingValue::Array {
                values: stroke_width,
            };
        }
        if stroke_cap.len() == len {
            mark.stroke_cap = EncodingValue::Array { values: stroke_cap };
        }
        if stroke_dash.len() == len {
            mark.stroke_dash = Some(EncodingValue::Array {
                values: stroke_dash,
            });
        }
        if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            mark.indices = Some(indices);
        }

        // Add gradients
        mark.gradients = gradients;

        Ok(SceneMark::Rule(mark))
    }
}
