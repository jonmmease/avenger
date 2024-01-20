use crate::error::VegaSceneGraphError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use serde::{Deserialize, Serialize};
use sg2d::marks::mark::SceneMark;
use sg2d::marks::trail::TrailMark;
use sg2d::value::EncodingValue;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaTrailItem {
    pub x: f32,
    pub y: f32,
    pub defined: Option<bool>,
    pub size: Option<f32>,
    pub fill: Option<String>,
    pub fill_opacity: Option<f32>,
    pub opacity: Option<f32>,
}

impl VegaMarkItem for VegaTrailItem {}

impl VegaMarkContainer<VegaTrailItem> {
    pub fn to_scene_graph(&self, origin: [f32; 2]) -> Result<SceneMark, VegaSceneGraphError> {
        // Get shape of first item and use that for all items for now
        let first = self.items.first();

        // Parse stroke color (Note, vega uses "fill" for the trail, but we use stroke
        let mut stroke = [0.0, 0.0, 0.0, 1.0];
        if let Some(item) = &first {
            if let Some(stroke_css) = &item.fill {
                let c = csscolorparser::parse(stroke_css)?;
                let base_opacity = item.opacity.unwrap_or(1.0);
                let stroke_opacity = c.a as f32 * item.fill_opacity.unwrap_or(1.0) * base_opacity;
                stroke = [c.r as f32, c.g as f32, c.b as f32, stroke_opacity]
            }
        }

        let mut mark = TrailMark {
            clip: self.clip,
            stroke,
            ..Default::default()
        };

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut size = Vec::<f32>::new();
        let mut defined = Vec::<bool>::new();

        for item in &self.items {
            x.push(item.x + origin[0]);
            y.push(item.y + origin[1]);
            if let Some(v) = item.size {
                size.push(v);
            }
            if let Some(v) = item.defined {
                defined.push(v);
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
        if size.len() == len {
            mark.size = EncodingValue::Array { values: size };
        }
        if defined.len() == len {
            mark.defined = EncodingValue::Array { values: defined };
        }

        Ok(SceneMark::Trail(mark))
    }
}
