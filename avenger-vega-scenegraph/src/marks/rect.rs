use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::values::CssColorOrGradient;
use avenger_common::types::{ColorOrGradient, Gradient};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaRectItem {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub x2: Option<f32>,
    pub y2: Option<f32>,
    pub fill: Option<CssColorOrGradient>,
    pub stroke: Option<CssColorOrGradient>,
    pub stroke_width: Option<f32>,
    pub corner_radius: Option<f32>,
    pub opacity: Option<f32>,
    pub fill_opacity: Option<f32>,
    pub stroke_opacity: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaRectItem {}

impl VegaMarkContainer<VegaRectItem> {
    pub fn to_scene_graph(&self, force_clip: bool) -> Result<SceneMark, AvengerVegaError> {
        let mut mark = SceneRectMark {
            clip: self.clip || force_clip,
            zindex: self.zindex,
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
        let mut fill = Vec::<ColorOrGradient>::new();
        let mut stroke = Vec::<ColorOrGradient>::new();
        let mut stroke_width = Vec::<f32>::new();
        let mut corner_radius = Vec::<f32>::new();
        let mut zindex = Vec::<i32>::new();
        let mut gradients = Vec::<Gradient>::new();

        // For each item, append explicit values to corresponding vector
        for item in &self.items {
            x.push(item.x.unwrap_or(0.0));
            y.push(item.y.unwrap_or(0.0));
            if let Some(v) = item.width {
                width.push(v);
            }
            if let Some(v) = item.height {
                height.push(v);
            }
            if let Some(v) = &item.fill {
                let opacity = item.fill_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                fill.push(v.to_color_or_grad(opacity, &mut gradients)?);
            }
            if let Some(v) = &item.stroke {
                let opacity = item.stroke_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                stroke.push(v.to_color_or_grad(opacity, &mut gradients)?);
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
            mark.x = ScalarOrArray::new_array(x);
        }
        if y.len() == len {
            mark.y = ScalarOrArray::new_array(y);
        }
        if width.len() == len {
            mark.width = Some(ScalarOrArray::new_array(width));
        }
        if height.len() == len {
            mark.height = Some(ScalarOrArray::new_array(height));
        }
        if fill.len() == len {
            mark.fill = ScalarOrArray::new_array(fill);
        }
        if stroke.len() == len {
            mark.stroke = ScalarOrArray::new_array(stroke);
        }
        if stroke_width.len() == len {
            mark.stroke_width = ScalarOrArray::new_array(stroke_width);
        }
        if corner_radius.len() == len {
            mark.corner_radius = ScalarOrArray::new_array(corner_radius);
        }
        if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            mark.indices = Some(Arc::new(indices));
        }

        // Add gradients
        mark.gradients = gradients;
        Ok(SceneMark::Rect(mark))
    }
}
