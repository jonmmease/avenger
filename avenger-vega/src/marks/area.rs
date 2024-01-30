use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::values::{CssColorOrGradient, StrokeDashSpec};
use avenger::marks::area::{AreaMark, AreaOrientation};
use avenger::marks::mark::SceneMark;
use avenger::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap, StrokeJoin};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaAreaItem {
    pub x: f32,
    pub y: f32,
    pub x2: Option<f32>,
    pub y2: Option<f32>,
    pub orient: Option<AreaOrientation>,
    pub defined: Option<bool>,
    pub fill: Option<CssColorOrGradient>,
    pub fill_opacity: Option<f32>,
    pub stroke_cap: Option<StrokeCap>,
    pub stroke_join: Option<StrokeJoin>,
    pub stroke: Option<CssColorOrGradient>,
    pub stroke_opacity: Option<f32>,
    pub stroke_width: Option<f32>,
    pub stroke_dash: Option<StrokeDashSpec>,
    pub opacity: Option<f32>,
}

impl VegaMarkItem for VegaAreaItem {}

impl VegaMarkContainer<VegaAreaItem> {
    pub fn to_scene_graph(&self) -> Result<SceneMark, AvengerVegaError> {
        // Get shape of first item and use that for all items for now
        let first = self.items.first();
        let stroke_cap = first.and_then(|item| item.stroke_cap).unwrap_or_default();
        let stroke_join = first.and_then(|item| item.stroke_join).unwrap_or_default();
        let orientation = first.and_then(|item| item.orient).unwrap_or_default();

        // Parse stroke color
        let mut stroke_width = 0.0;
        let mut stroke = ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]);
        let mut fill = ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]);
        let mut stroke_dash: Option<Vec<f32>> = None;
        let mut gradients = Vec::<Gradient>::new();

        if let Some(item) = &first {
            if let Some(stroke_css) = &item.stroke {
                let base_opacity = item.opacity.unwrap_or(1.0);
                let stroke_opacity = item.stroke_opacity.unwrap_or(1.0) * base_opacity;
                stroke = stroke_css.to_color_or_grad(stroke_opacity, &mut gradients)?;
                stroke_width = item.stroke_width.unwrap_or(1.0);
            }
            if let Some(d) = &item.stroke_dash {
                stroke_dash = Some(d.to_array()?.to_vec());
            }
            if let Some(fill_css) = &item.fill {
                let base_opacity = item.opacity.unwrap_or(1.0);
                let fill_opacity = item.fill_opacity.unwrap_or(1.0) * base_opacity;
                fill = fill_css.to_color_or_grad(fill_opacity, &mut gradients)?;
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
            x.push(item.x);
            y.push(item.y);
            if let Some(v) = &item.x2 {
                x2.push(*v);
            }
            if let Some(v) = &item.y2 {
                y2.push(*v);
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
        mark.gradients = gradients;

        Ok(SceneMark::Area(mark))
    }
}
