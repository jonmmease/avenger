use std::{f32::consts::PI, sync::Arc};

use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
use avenger_text::types::{FontStyle, FontWeight, TextAlign, TextBaseline};
use serde::{Deserialize, Serialize};

use crate::{
    error::AvengerVegaError,
    marks::{
        mark::{VegaMarkContainer, VegaMarkItem},
        values::MissingNullOrValue,
    },
};

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
    pub line_height: Option<f32>,
    pub font_weight: Option<FontWeight>,
    pub font_style: Option<FontStyle>,
    pub limit: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaTextItem {}

impl VegaTextItem {
    // Match vega-scenegraph/src/util/text.js offset().
    fn baseline_offset(&self) -> f32 {
        let size = self.font_size.unwrap_or(11.0);
        let line_height = self.line_height.unwrap_or(size + 2.0);
        let offset = match self.baseline.unwrap_or(TextBaseline::Alphabetic) {
            TextBaseline::Top => 0.79 * size,
            TextBaseline::Middle => 0.30 * size,
            TextBaseline::Bottom => -0.21 * size,
            TextBaseline::LineTop => 0.29 * size + 0.5 * line_height,
            TextBaseline::LineBottom => 0.29 * size - 0.5 * line_height,
            TextBaseline::Alphabetic => 0.0,
        };
        // Vega rounds baseline offsets with JavaScript Math.round.
        (offset + 0.5).floor()
    }
}

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
                let [r, g, b, a] = [c.r as f32, c.g as f32, c.b as f32, c.a as f32];
                let opacity = a * item.fill_opacity.unwrap_or(1.0) * item.opacity.unwrap_or(1.0);
                color.push(ColorOrGradient::Color([r, g, b, opacity]))
            }

            // Compute x and y
            let mut item_x = item.x.unwrap_or(0.0);
            let mut item_y = item.y.unwrap_or(0.0);
            if let (Some(radius), Some(theta)) = (item.radius, item.theta) {
                item_x += radius * f32::cos(theta - PI / 2.0);
                item_y += radius * f32::sin(theta - PI / 2.0);
            }
            x.push(item_x);
            y.push(item_y);
            // Vega offsets rotate with the label; scene-mark offsets use canvas coordinates.
            let item_dx = item.dx.unwrap_or(0.0);
            let item_dy = item.dy.unwrap_or(0.0) + item.baseline_offset();
            let (sin, cos) = item.angle.unwrap_or(0.0).to_radians().sin_cos();
            dx.push(item_dx * cos - item_dy * sin);
            dy.push(item_dx * sin + item_dy * cos);
            text.push(match item.text.clone() {
                Some(serde_json::Value::String(s)) => s,
                Some(serde_json::Value::Null) | None => "".to_string(),
                Some(v) => v.to_string(),
            });

            if let Some(v) = item.align {
                align.push(v);
            }

            if let Some(v) = item.angle {
                angle.push(v);
            }

            if let Some(v) = &item.font {
                font.push(v.clone());
            }

            font_size.push(item.font_size.unwrap_or(11.0));

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
        if dx.len() == len {
            mark.dx = ScalarOrArray::new_array(dx);
        }
        if dy.len() == len {
            mark.dy = ScalarOrArray::new_array(dy);
        }
        if text.len() == len {
            mark.text = ScalarOrArray::new_array(text);
        }
        if align.len() == len {
            mark.align = ScalarOrArray::new_array(align);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vega_baselines_import_as_alphabetic_anchors() {
        for (baseline, size, line_height, offset) in [
            (None, Some(30.0), None, 0.0),
            (Some(TextBaseline::Alphabetic), Some(10.0), None, 0.0),
            (Some(TextBaseline::Top), Some(10.0), None, 8.0),
            (Some(TextBaseline::Middle), Some(10.0), None, 3.0),
            (Some(TextBaseline::Bottom), Some(10.0), None, -2.0),
            (Some(TextBaseline::LineTop), Some(10.0), None, 9.0),
            (Some(TextBaseline::LineBottom), Some(10.0), None, -3.0),
            (Some(TextBaseline::LineTop), Some(10.0), Some(20.0), 13.0),
            (Some(TextBaseline::LineBottom), Some(10.0), Some(20.0), -7.0),
            (Some(TextBaseline::Bottom), Some(50.0), None, -10.0),
            (Some(TextBaseline::Top), Some(13.5), None, 11.0),
            (Some(TextBaseline::Top), None, None, 9.0),
        ] {
            let container = VegaMarkContainer {
                items: vec![VegaTextItem {
                    y: Some(20.0),
                    font_size: size,
                    line_height,
                    baseline,
                    text: Some("label".into()),
                    ..Default::default()
                }],
                ..Default::default()
            };
            let SceneMark::Text(mark) = container.to_scene_graph(false).unwrap() else {
                panic!("expected text mark");
            };
            assert_eq!(*mark.y_iter().next().unwrap(), 20.0);
            assert_eq!(*mark.dy_iter().next().unwrap(), offset);
            assert_eq!(
                *mark.baseline_iter().next().unwrap(),
                TextBaseline::Alphabetic
            );
            assert_eq!(*mark.font_size_iter().next().unwrap(), size.unwrap_or(11.0));
        }
    }

    #[test]
    fn rotated_baselines_and_offsets_preserve_the_vega_anchor() {
        let container = VegaMarkContainer {
            items: vec![VegaTextItem {
                x: Some(100.0),
                y: Some(200.0),
                radius: Some(10.0),
                theta: Some(PI / 2.0),
                dx: Some(3.0),
                dy: Some(4.0),
                angle: Some(90.0),
                baseline: Some(TextBaseline::Top),
                font_size: Some(10.0),
                text: Some("label".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let SceneMark::Text(mark) = container.to_scene_graph(false).unwrap() else {
            panic!("expected text mark");
        };
        assert_eq!(*mark.x_iter().next().unwrap(), 110.0);
        assert_eq!(*mark.y_iter().next().unwrap(), 200.0);
        assert!((*mark.dx_iter().next().unwrap() + 12.0).abs() < 1e-5);
        assert!((*mark.dy_iter().next().unwrap() - 3.0).abs() < 1e-5);
        assert_eq!(*mark.angle_iter().next().unwrap(), 90.0);
    }
}
