use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::values::CssColorOrGradient;
use avenger_common::types::{ColorOrGradient, Gradient};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::trail::SceneTrailMark;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaTrailItem {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub defined: Option<bool>,
    pub size: Option<f32>,
    pub fill: Option<CssColorOrGradient>,
    pub fill_opacity: Option<f32>,
    pub opacity: Option<f32>,
}

impl VegaMarkItem for VegaTrailItem {}

impl VegaMarkContainer<VegaTrailItem> {
    pub fn to_scene_graph(&self, force_clip: bool) -> Result<SceneMark, AvengerVegaError> {
        // Get shape of first item and use that for all items for now
        let first = self.items.first();
        let mut gradients = Vec::<Gradient>::new();

        // Parse stroke color (Note, vega uses "fill" for the trail, but we use stroke
        let mut stroke = ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]);
        if let Some(item) = &first {
            if let Some(stroke_css) = &item.fill {
                let base_opacity = item.opacity.unwrap_or(1.0);
                let stroke_opacity = item.fill_opacity.unwrap_or(1.0) * base_opacity;
                stroke = stroke_css.to_color_or_grad(stroke_opacity, &mut gradients)?;
            }
        }

        let mut mark = SceneTrailMark {
            clip: self.clip || force_clip,
            zindex: self.zindex,
            gradients,
            stroke,
            ..Default::default()
        };

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut size = Vec::<f32>::new();
        let mut defined = Vec::<bool>::new();

        for item in &self.items {
            x.push(item.x.unwrap_or(0.0));
            y.push(item.y.unwrap_or(0.0));
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
            mark.x = ScalarOrArray::Array(x);
        }
        if y.len() == len {
            mark.y = ScalarOrArray::Array(y);
        }
        if size.len() == len {
            mark.size = ScalarOrArray::Array(size);
        }
        if defined.len() == len {
            mark.defined = ScalarOrArray::Array(defined);
        }

        Ok(SceneMark::Trail(mark))
    }
}
