use crate::error::VegaSceneGraphError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::rule::parse_dash_str;
use serde::{Deserialize, Serialize};
use sg2d::marks::line::LineMark;
use sg2d::marks::mark::SceneMark;
use sg2d::value::{EncodingValue, StrokeCap, StrokeJoin};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaLineItem {
    pub x: f32,
    pub y: f32,
    pub defined: Option<bool>,
    pub stroke_cap: Option<StrokeCap>,
    pub stroke_join: Option<StrokeJoin>,
    pub stroke: Option<String>,
    pub stroke_opacity: Option<f32>,
    pub stroke_width: Option<f32>,
    pub stroke_dash: Option<String>,
    pub opacity: Option<f32>,
}

impl VegaMarkItem for VegaLineItem {}

impl VegaMarkContainer<VegaLineItem> {
    pub fn to_scene_graph(&self, origin: [f32; 2]) -> Result<SceneMark, VegaSceneGraphError> {
        // Get shape of first item and use that for all items for now
        let first = self.items.first();
        let stroke_width = first.and_then(|item| item.stroke_width).unwrap_or(1.0);
        let stroke_cap = first.and_then(|item| item.stroke_cap).unwrap_or_default();
        let stroke_join = first.and_then(|item| item.stroke_join).unwrap_or_default();

        // Parse stroke color
        let mut stroke = [0.0, 0.0, 0.0, 1.0];
        let mut stroke_dash: Option<Vec<f32>> = None;
        if let Some(item) = &first {
            if let Some(stroke_css) = &item.stroke {
                let c = csscolorparser::parse(&stroke_css)?;
                let base_opacity = item.opacity.unwrap_or(1.0);
                let stroke_opacity = c.a as f32 * item.stroke_opacity.unwrap_or(1.0) * base_opacity;
                stroke = [c.r as f32, c.g as f32, c.b as f32, stroke_opacity]
            }
            if let Some(d) = &item.stroke_dash {
                stroke_dash = Some(parse_dash_str(&d)?);
            }
        }

        let mut mark = LineMark {
            clip: self.clip,
            stroke,
            stroke_width,
            stroke_cap,
            stroke_dash,
            stroke_join,
            ..Default::default()
        };

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut defined = Vec::<bool>::new();

        for item in &self.items {
            x.push(item.x + origin[0]);
            y.push(item.y + origin[1]);
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
        if defined.len() == len {
            mark.defined = EncodingValue::Array { values: defined };
        }

        Ok(SceneMark::Line(mark))
    }
}
