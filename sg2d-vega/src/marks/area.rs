use crate::error::VegaSceneGraphError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::values::StrokeDashSpec;
use serde::{Deserialize, Serialize};
use sg2d::marks::area::{AreaMark, AreaOrientation};
use sg2d::marks::mark::SceneMark;
use sg2d::value::{EncodingValue, StrokeCap, StrokeJoin};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaAreaItem {
    pub x: f32,
    pub y: f32,
    pub x2: Option<f32>,
    pub y2: Option<f32>,
    pub orient: Option<AreaOrientation>,
    pub defined: Option<bool>,
    pub fill: Option<String>,
    pub fill_opacity: Option<f32>,
    pub stroke_cap: Option<StrokeCap>,
    pub stroke_join: Option<StrokeJoin>,
    pub stroke: Option<String>,
    pub stroke_opacity: Option<f32>,
    pub stroke_width: Option<f32>,
    pub stroke_dash: Option<StrokeDashSpec>,
    pub opacity: Option<f32>,
}

impl VegaMarkItem for VegaAreaItem {}

impl VegaMarkContainer<VegaAreaItem> {
    pub fn to_scene_graph(&self, origin: [f32; 2]) -> Result<SceneMark, VegaSceneGraphError> {
        // Get shape of first item and use that for all items for now
        let first = self.items.first();
        let stroke_cap = first.and_then(|item| item.stroke_cap).unwrap_or_default();
        let stroke_join = first.and_then(|item| item.stroke_join).unwrap_or_default();
        let orientation = first.and_then(|item| item.orient).unwrap_or_default();

        // Parse stroke color
        let mut stroke_width = 0.0;
        let mut stroke = [0.0, 0.0, 0.0, 1.0];
        let mut fill = [0.0, 0.0, 0.0, 0.0];
        let mut stroke_dash: Option<Vec<f32>> = None;

        if let Some(item) = &first {
            if let Some(stroke_css) = &item.stroke {
                let c = csscolorparser::parse(stroke_css)?;
                let base_opacity = item.opacity.unwrap_or(1.0);
                let stroke_opacity = c.a as f32 * item.stroke_opacity.unwrap_or(1.0) * base_opacity;
                stroke = [c.r as f32, c.g as f32, c.b as f32, stroke_opacity];
                stroke_width = item.stroke_width.unwrap_or(1.0);
            }
            if let Some(d) = &item.stroke_dash {
                stroke_dash = Some(d.to_array()?.to_vec());
            }
            if let Some(fill_css) = &item.fill {
                let c = csscolorparser::parse(fill_css)?;
                let base_opacity = item.opacity.unwrap_or(1.0);
                let fill_opacity = c.a as f32 * item.fill_opacity.unwrap_or(1.0) * base_opacity;
                fill = [c.r as f32, c.g as f32, c.b as f32, fill_opacity]
            }
        }

        let mut mark = AreaMark {
            clip: self.clip,
            orientation,
            fill,
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
        let mut x2 = Vec::<f32>::new();
        let mut y2 = Vec::<f32>::new();
        let mut defined = Vec::<bool>::new();

        for item in &self.items {
            x.push(item.x + origin[0]);
            y.push(item.y + origin[1]);
            if let Some(v) = &item.x2 {
                x2.push(*v + origin[0]);
            }
            if let Some(v) = &item.y2 {
                y2.push(*v + origin[1]);
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
        if x2.len() == len {
            mark.x2 = EncodingValue::Array { values: x2 };
        }
        if y2.len() == len {
            mark.y2 = EncodingValue::Array { values: y2 };
        }
        if defined.len() == len {
            mark.defined = EncodingValue::Array { values: defined };
        }

        Ok(SceneMark::Area(mark))
    }
}
