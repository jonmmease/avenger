use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::values::MissingNullOrValue;
use avenger_common::types::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_text::types::{FontStyle, FontWeight, TextAlign, TextBaseline};
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
use std::sync::Arc;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaTextItem {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub text: Option<serde_json::Value>,

    // Optional
    pub radius: Option<f32>,
    pub theta: Option<f32>,
    pub align: Option<TextAlign>,
    pub angle: Option<f32>,
    pub baseline: Option<TextBaseline>,
    pub dx: Option<f32>,
    pub dy: Option<f32>,
    pub fill: MissingNullOrValue<String>,
    pub opacity: Option<f32>,
    pub fill_opacity: Option<f32>,
    pub font: Option<String>,
    pub font_size: Option<f32>,
    pub font_weight: Option<FontWeight>,
    pub font_style: Option<FontStyle>,
    pub limit: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaTextItem {}

impl VegaMarkContainer<VegaTextItem> {
    pub fn to_scene_graph(&self, force_clip: bool) -> Result<SceneMark, AvengerVegaError> {
        // Init mark with scalar defaults
        let mut mark = SceneTextMark {
            clip: self.clip || force_clip,
            zindex: self.zindex,
            ..Default::default()
        };
        if let Some(name) = &self.name {
            mark.name = name.clone();
        }

        // Init vector for each encoding channel
        let mut text = Vec::<String>::new();
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut align = Vec::<TextAlign>::new();
        let mut baseline = Vec::<TextBaseline>::new();
        let mut angle = Vec::<f32>::new();
        let mut color = Vec::<ColorOrGradient>::new();
        let mut dx = Vec::<f32>::new();
        let mut dy = Vec::<f32>::new();
        let mut font = Vec::<String>::new();
        let mut font_size = Vec::<f32>::new();
        let mut font_weight = Vec::<FontWeight>::new();
        let mut font_style = Vec::<FontStyle>::new();
        let mut limit = Vec::<f32>::new();
        let mut zindex = Vec::<i32>::new();

        let mut len: usize = 0;
        for item in &self.items {
            // When fill is set to null literal (not just missing) we skip the
            // text item all together
            if item.fill.is_null() {
                continue;
            }
            if let Some(v) = item.fill.as_option() {
                let c = csscolorparser::parse(v)?;
                let opacity =
                    c.a as f32 * item.fill_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                color.push(ColorOrGradient::Color([
                    c.r as f32, c.g as f32, c.b as f32, opacity,
                ]))
            }

            // Compute x and y
            let mut item_x = item.x.unwrap_or(0.0);
            let mut item_y = item.y.unwrap_or(0.0);
            if let (Some(radius), Some(theta)) = (item.radius, item.theta) {
                item_x += radius * f32::cos(theta - PI / 2.0);
                item_y += radius * f32::sin(theta - PI / 2.0);
            }
            item_x += item.dx.unwrap_or(0.0);
            item_y += item.dy.unwrap_or(0.0);
            x.push(item_x);
            y.push(item_y);
            text.push(match item.text.clone() {
                Some(serde_json::Value::String(s)) => s,
                Some(serde_json::Value::Null) | None => "".to_string(),
                Some(v) => v.to_string(),
            });

            if let Some(v) = item.align {
                align.push(v);
            }

            if let Some(v) = item.baseline {
                baseline.push(v);
            }

            if let Some(v) = item.angle {
                angle.push(v);
            }

            if let Some(v) = item.dx {
                dx.push(v);
            }

            if let Some(v) = item.dy {
                dy.push(v);
            }

            if let Some(v) = &item.font {
                font.push(v.clone());
            }

            if let Some(v) = item.font_size {
                font_size.push(v);
            }

            if let Some(v) = item.font_weight {
                font_weight.push(v);
            }

            if let Some(v) = item.font_style {
                font_style.push(v);
            }

            if let Some(v) = item.limit {
                limit.push(v);
            }

            if let Some(v) = item.zindex {
                zindex.push(v);
            }

            len += 1;
        }

        // Update len
        mark.len = len as u32;

        // Override values with vectors
        if x.len() == len {
            mark.x = ScalarOrArray::new_array(x);
        }
        if y.len() == len {
            mark.y = ScalarOrArray::new_array(y);
        }
        if text.len() == len {
            mark.text = ScalarOrArray::new_array(text);
        }
        if align.len() == len {
            mark.align = ScalarOrArray::new_array(align);
        }
        if baseline.len() == len {
            mark.baseline = ScalarOrArray::new_array(baseline);
        }
        if angle.len() == len {
            mark.angle = ScalarOrArray::new_array(angle);
        }
        if color.len() == len {
            mark.color = ScalarOrArray::new_array(color);
        }
        if font.len() == len {
            mark.font = ScalarOrArray::new_array(font);
        }
        if font_size.len() == len {
            mark.font_size = ScalarOrArray::new_array(font_size);
        }
        if font_weight.len() == len {
            mark.font_weight = ScalarOrArray::new_array(font_weight);
        }
        if font_style.len() == len {
            mark.font_style = ScalarOrArray::new_array(font_style);
        }
        if limit.len() == len {
            mark.limit = ScalarOrArray::new_array(limit);
        }
        if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            mark.indices = Some(Arc::new(indices));
        }
        Ok(SceneMark::Text(Arc::new(mark)))
    }
}
