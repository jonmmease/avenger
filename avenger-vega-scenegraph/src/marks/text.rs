use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::values::MissingNullOrValue;
use avenger_color::ColorOrGradient;

use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_text::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_text::{LabelAlign, LabelLineHeight, LabelWidth};
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
use std::sync::Arc;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaTextItem {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub text: Option<serde_json::Value>,
    /// What splits string text into lines: a string, or a regular expression, which arrives as
    /// an empty object and splits nothing.
    pub line_break: Option<serde_json::Value>,

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

    /// The item's text as plain text whose newlines end lines: its lines as Vega's `textLines`
    /// reads them, each with its own newlines as spaces, as Vega draws them.
    fn text(&self) -> String {
        use serde_json::Value;
        let line = |value: &Value| match value {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let lines: Vec<String> = match (&self.text, &self.line_break) {
            (Some(Value::Array(values)), _) if values.len() > 1 => {
                values.iter().map(line).collect()
            }
            (Some(Value::Array(values)), _) => vec![values.first().map_or(String::new(), line)],
            (Some(Value::String(text)), Some(Value::String(line_break)))
                if !text.is_empty() && !line_break.is_empty() =>
            {
                text.split(line_break.as_str())
                    .map(str::to_string)
                    .collect()
            }
            (Some(text), _) => vec![line(text)],
            (None, _) => vec![String::new()],
        };
        let mut text = lines
            .iter()
            .map(|line| line.replace(is_newline, " "))
            .collect::<Vec<_>>()
            .join("\n");
        // A final newline starts no line, but Vega draws a final empty one.
        if lines.len() > 1 && lines.last().is_some_and(String::is_empty) {
            text.push('\n');
        }
        text
    }
}

/// Whether a character ends a line, as the label crate reads newlines.
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\u{000B}' | '\u{000C}' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
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
        let mut font = Vec::<String>::new();
        let mut font_size = Vec::<f32>::new();
        let mut font_weight = Vec::<FontWeight>::new();
        let mut font_style = Vec::<FontStyle>::new();
        let mut width = Vec::<LabelWidth>::new();
        let mut line_height = Vec::<LabelLineHeight>::new();
        let mut line_align = Vec::<LabelAlign>::new();
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
            // Convert Vega's baseline and local offsets to an alphabetic anchor.
            // Rotate the offset too, preserving the original rotation pivot.
            let dx = item.dx.unwrap_or(0.0);
            let dy = item.dy.unwrap_or(0.0) + item.baseline_offset();
            let (sin, cos) = item.angle.unwrap_or(0.0).to_radians().sin_cos();
            item_x += cos * dx - sin * dy;
            item_y += sin * dx + cos * dy;
            x.push(item_x);
            y.push(item_y);
            text.push(item.text());
            // Vega's lineHeight is the distance between baselines.
            let size = item.font_size.unwrap_or(11.0);
            line_height.push(LabelLineHeight::Fixed(
                item.line_height.unwrap_or(size + 2.0),
            ));

            if let Some(v) = item.align {
                align.push(v);
            }
            // Vega aligns each line at x, as the anchor does.
            line_align.push(match item.align.unwrap_or_default() {
                TextAlign::Left => LabelAlign::Left,
                TextAlign::Center => LabelAlign::Center,
                TextAlign::Right => LabelAlign::Right,
            });

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

            // A limit cuts each line to it, and never wraps.
            width.push(match item.limit {
                Some(limit) if limit > 0.0 => LabelWidth::Max(limit),
                _ => LabelWidth::Auto,
            });

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
        mark.text_syntax = TextSyntaxMode::PlainLines;
        mark.line_height = ScalarOrArray::new_array(line_height);
        mark.width = ScalarOrArray::new_array(width);
        mark.wrap = false;
        mark.ellipsis = true;
        mark.line_align = ScalarOrArray::new_array(line_align);
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
            assert_eq!(*mark.y_iter().next().unwrap(), 20.0 + offset);
            assert_eq!(
                *mark.baseline_iter().next().unwrap(),
                TextBaseline::Alphabetic
            );
            assert_eq!(*mark.font_size_iter().next().unwrap(), size.unwrap_or(11.0));
        }
    }

    /// The mark that one item imports as.
    fn import(item: VegaTextItem) -> Arc<SceneTextMark> {
        let container = VegaMarkContainer {
            items: vec![item],
            ..Default::default()
        };
        let SceneMark::Text(mark) = container.to_scene_graph(false).unwrap() else {
            panic!("expected text mark");
        };
        mark
    }

    #[test]
    fn vega_text_lines_import_as_plain_lines() {
        use serde_json::json;
        let text = |text: serde_json::Value, line_break: Option<serde_json::Value>| {
            let mark = import(VegaTextItem {
                text: Some(text),
                line_break,
                ..Default::default()
            });
            assert_eq!(mark.text_syntax, TextSyntaxMode::PlainLines);
            let text = mark.text_iter().next().unwrap().clone();
            text
        };
        // An array of several elements is lines, of one element a line, and nulls are empty.
        assert_eq!(
            text(json!(["Revenue", "by region"]), None),
            "Revenue\nby region"
        );
        assert_eq!(text(json!(["Revenue"]), None), "Revenue");
        assert_eq!(text(json!(["a", null, 3]), None), "a\n\n3");
        // A string lineBreak splits string text, and a regular expression, which arrives as an
        // object, splits nothing.
        assert_eq!(text(json!("a|b"), Some(json!("|"))), "a\nb");
        assert_eq!(text(json!("a|b"), Some(json!({}))), "a|b");
        // Newlines within a line are spaces, as Vega draws them.
        assert_eq!(text(json!("a\nb"), None), "a b");
        assert_eq!(text(json!(["a\r\nb", "c"]), None), "a  b\nc");
        // A trailing empty line still counts.
        assert_eq!(text(json!(["a", ""]), None), "a\n\n");
    }

    #[test]
    fn vega_line_heights_space_baselines() {
        let line_height = |font_size, line_height| {
            let mark = import(VegaTextItem {
                text: Some("a".into()),
                font_size,
                line_height,
                ..Default::default()
            });
            let line_height = *mark.line_height_iter().next().unwrap();
            line_height
        };
        assert_eq!(line_height(None, None), LabelLineHeight::Fixed(13.0));
        assert_eq!(line_height(Some(20.0), None), LabelLineHeight::Fixed(22.0));
        assert_eq!(
            line_height(Some(20.0), Some(30.0)),
            LabelLineHeight::Fixed(30.0)
        );
    }

    #[test]
    fn vega_limits_cut_each_line() {
        let mark = import(VegaTextItem {
            text: Some(serde_json::json!([
                "Revenue by region",
                "in millions of dollars",
                "USD"
            ])),
            limit: Some(60.0),
            font_size: Some(12.0),
            ..Default::default()
        });
        let pdf = avenger_text::default_text_engine()
            .extract_pdf(&avenger_text::pdf::TextPdfExtractionConfig {
                text: mark.text_iter().next().unwrap(),
                color: [0.0, 0.0, 0.0, 1.0],
                font: "sans-serif",
                font_size: 12.0,
                font_weight: FontWeight::default(),
                font_style: FontStyle::Normal,
                layout: mark.layout_iter().next().unwrap(),
                syntax_mode: mark.text_syntax,
                params: avenger_text::empty_label_params(),
                number_format: None,
                datetime_format: None,
            })
            .unwrap();
        let lines: Vec<_> = pdf.semantic_text.split('\n').collect();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].ends_with('…') && lines[1].ends_with('…'),
            "{lines:?}"
        );
        assert_eq!(lines[2], "USD");
        assert!(pdf.bounds.width <= 60.0, "{:?}", pdf.bounds);
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
        assert!((*mark.x_iter().next().unwrap() - 98.0).abs() < 1e-5);
        assert!((*mark.y_iter().next().unwrap() - 203.0).abs() < 1e-5);
        assert_eq!(*mark.angle_iter().next().unwrap(), 90.0);
    }
}
