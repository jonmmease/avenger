use crate::error::VegaSceneGraphError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use serde::{Deserialize, Serialize};
use sg2d::marks::mark::SceneMark;
use sg2d::marks::rule::RuleMark;
use sg2d::value::{EncodingValue, StrokeCap};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaRuleItem {
    pub x: f32,
    pub y: f32,
    pub x2: Option<f32>,
    pub y2: Option<f32>,
    pub stroke: Option<String>,
    pub stroke_width: Option<f32>,
    pub stroke_cap: Option<StrokeCap>,
    pub stroke_opacity: Option<f32>,
    pub opacity: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaRuleItem {}

impl VegaMarkContainer<VegaRuleItem> {
    pub fn to_scene_graph(&self, origin: [f32; 2]) -> Result<SceneMark, VegaSceneGraphError> {
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
        let mut stroke = Vec::<[f32; 4]>::new();
        let mut stroke_width = Vec::<f32>::new();
        let mut stroke_cap = Vec::<StrokeCap>::new();
        let mut zindex = Vec::<i32>::new();

        // For each item, append explicit values to corresponding vector
        for item in &self.items {
            x0.push(item.x + origin[0]);
            y0.push(item.y + origin[1]);
            x1.push(item.x2.unwrap_or(item.x) + origin[0]);
            y1.push(item.y2.unwrap_or(item.y) + origin[1]);

            if let Some(s) = &item.stroke {
                let c = csscolorparser::parse(s)?;
                let opacity = item.stroke_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                stroke.push([c.r as f32, c.g as f32, c.b as f32, opacity]);
            }

            if let Some(s) = item.stroke_width {
                stroke_width.push(s);
            }

            if let Some(s) = item.stroke_cap {
                stroke_cap.push(s);
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
        if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            mark.indices = Some(indices);
        }

        Ok(SceneMark::Rule(mark))
    }
}
