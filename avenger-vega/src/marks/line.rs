use super::values::{CssColorOrGradient, StrokeDashSpec};
use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use avenger_common::types::{ColorOrGradient, Gradient, StrokeCap, StrokeJoin};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::mark::SceneMark;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaLineItem {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub defined: Option<bool>,
    pub stroke_cap: Option<StrokeCap>,
    pub stroke_join: Option<StrokeJoin>,
    pub stroke: Option<CssColorOrGradient>,
    pub stroke_opacity: Option<f32>,
    pub stroke_width: Option<f32>,
    pub stroke_dash: Option<StrokeDashSpec>,
    pub opacity: Option<f32>,
}

impl VegaMarkItem for VegaLineItem {}

impl VegaMarkContainer<VegaLineItem> {
    pub fn to_scene_graph(&self, force_clip: bool) -> Result<SceneMark, AvengerVegaError> {
        // Get shape of first item and use that for all items for now
        let first = self.items.first();
        let stroke_width = first.and_then(|item| item.stroke_width).unwrap_or(1.0);
        let stroke_cap = first.and_then(|item| item.stroke_cap).unwrap_or_default();
        let stroke_join = first.and_then(|item| item.stroke_join).unwrap_or_default();
        let mut gradients = Vec::<Gradient>::new();

        // Parse stroke color
        let mut stroke = ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]);
        let mut stroke_dash: Option<Vec<f32>> = None;
        if let Some(item) = &first {
            if let Some(stroke_css) = &item.stroke {
                let base_opacity = item.opacity.unwrap_or(1.0);
                let stroke_opacity = item.stroke_opacity.unwrap_or(1.0) * base_opacity;
                stroke = stroke_css.to_color_or_grad(stroke_opacity, &mut gradients)?;
            }
            if let Some(d) = &item.stroke_dash {
                stroke_dash = Some(d.to_array()?.to_vec());
            }
        }

        let mut mark = SceneLineMark {
            clip: self.clip || force_clip,
            zindex: self.zindex,
            stroke,
            stroke_width,
            stroke_cap,
            stroke_dash,
            stroke_join,
            gradients,
            ..Default::default()
        };

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut defined = Vec::<bool>::new();

        for item in &self.items {
            x.push(item.x.unwrap_or(0.0));
            y.push(item.y.unwrap_or(0.0));
            if let Some(v) = item.defined {
                defined.push(v);
            }
        }

        // Override values with vectors
        let len = self.items.len();
        mark.len = len as u32;

        if x.len() == len {
            mark.x = ScalarOrArray::Array(Arc::new(x));
        }
        if y.len() == len {
            mark.y = ScalarOrArray::Array(Arc::new(y));
        }
        if defined.len() == len {
            mark.defined = ScalarOrArray::Array(Arc::new(defined));
        }

        Ok(SceneMark::Line(mark))
    }
}
