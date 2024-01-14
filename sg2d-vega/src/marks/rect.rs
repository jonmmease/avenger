use crate::error::VegaSceneGraphError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use serde::{Deserialize, Serialize};
use sg2d::marks::mark::SceneMark;
use sg2d::marks::rect::RectMark;
use sg2d::value::EncodingValue;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaRectItem {
    pub x: f32,
    pub y: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub x2: Option<f32>,
    pub y2: Option<f32>,
    pub fill: Option<String>,
    pub stroke: Option<String>,
    pub stroke_width: Option<f32>,
    pub corner_radius: Option<f32>,
    pub opacity: Option<f32>,
    pub fill_opacity: Option<f32>,
    pub stroke_opacity: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaRectItem {}

impl VegaMarkContainer<VegaRectItem> {
    pub fn to_scene_graph(&self, origin: [f32; 2]) -> Result<SceneMark, VegaSceneGraphError> {
        let mut mark = RectMark {
            clip: self.clip,
            ..Default::default()
        };

        if let Some(name) = &self.name {
            mark.name = name.clone();
        }

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut width = Vec::<f32>::new();
        let mut height = Vec::<f32>::new();
        let mut fill = Vec::<[f32; 4]>::new();
        let mut stroke = Vec::<[f32; 4]>::new();
        let mut stroke_width = Vec::<f32>::new();
        let mut corner_radius = Vec::<f32>::new();
        let mut zindex = Vec::<i32>::new();

        // For each item, append explicit values to corresponding vector
        for item in &self.items {
            x.push(item.x + origin[0]);
            y.push(item.y + origin[1]);
            if let Some(v) = item.width {
                width.push(v);
            }
            if let Some(v) = item.height {
                height.push(v);
            }
            if let Some(v) = &item.fill {
                let c = csscolorparser::parse(v)?;
                let opacity = item.fill_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                fill.push([c.r as f32, c.g as f32, c.b as f32, opacity])
            }
            if let Some(v) = &item.stroke {
                let c = csscolorparser::parse(v)?;
                let opacity = item.stroke_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                stroke.push([c.r as f32, c.g as f32, c.b as f32, opacity])
            }
            if let Some(v) = item.stroke_width {
                stroke_width.push(v);
            }
            if let Some(v) = item.corner_radius {
                corner_radius.push(v);
            }
            if let Some(v) = item.zindex {
                zindex.push(v);
            }
        }

        // Override values with vectors
        let len = self.items.len();
        mark.len = len as u32;

        if x.len() == len {
            mark.x = EncodingValue::Array { values: x };
        }
        if y.len() == len {
            mark.y = EncodingValue::Array { values: y };
        }
        if width.len() == len {
            mark.width = EncodingValue::Array { values: width };
        }
        if height.len() == len {
            mark.height = EncodingValue::Array { values: height };
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
        if corner_radius.len() == len {
            mark.corner_radius = EncodingValue::Array {
                values: corner_radius,
            };
        }
        if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            mark.indices = Some(indices);
        }

        Ok(SceneMark::Rect(mark))
    }
}
