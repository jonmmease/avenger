//! Line swatches: each entry's line segment and label.

use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use avenger_geometry::{marks::TextGeometryUtils, rtree::EnvelopeUtils};
use avenger_scenegraph::marks::{
    group::SceneGroup, line::SceneLineMark, mark::SceneMark, text::SceneTextMark,
};
use avenger_typst_label::LabelEngine;

use super::{make_entry_label, make_hit_rect, LegendStyle};

/// Each entry's line: a scalar for every entry, or an array with one value per entry.
#[derive(Debug, Clone)]
pub struct LineSwatches {
    pub stroke_width: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_dash: ScalarOrArray<Option<Vec<f32>>>,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    /// The length of each entry's line.
    pub line_length: ScalarOrArray<f32>,
}

impl Default for LineSwatches {
    fn default() -> Self {
        Self {
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke_dash: None.into(),
            stroke_width: 2.0.into(),
            stroke_cap: StrokeCap::Butt,
            stroke_join: StrokeJoin::Miter,
            line_length: ScalarOrArray::new_scalar(10.0),
        }
    }
}

impl LineSwatches {
    /// The encodings' lengths, which must agree where they are arrays.
    pub(crate) fn lengths(&self) -> [usize; 3] {
        [
            self.stroke.len(),
            self.stroke_width.len(),
            self.stroke_dash.len(),
        ]
    }

    /// The row height, which fits the tallest label, and one entry group per label, each with
    /// its origin at the middle of its row's start: a hit rect, its line, and its label after
    /// the longest line.
    pub(crate) fn entries(
        &self,
        texts: &[String],
        text_padding: f32,
        style: &LegendStyle,
        engine: &LabelEngine,
    ) -> (f32, Vec<SceneGroup>) {
        let len = texts.len();
        let row_height = SceneTextMark {
            text: texts.to_vec().into(),
            font: style.label_font_family.clone().into(),
            font_size: style.label_font_size.into(),
            font_weight: style.label_font_weight.into(),
            text_syntax: style.label_syntax,
            ..Default::default()
        }
        .bounding_box(engine)
        .height();
        let stroke_widths = self.stroke_width.as_vec(len, None);
        let stroke_dashes = self.stroke_dash.as_vec(len, None);
        let strokes = self.stroke.as_vec(len, None);
        let line_lengths = self.line_length.as_vec(len, None);
        // Labels start after the longest line, so that they align.
        let text_x = line_lengths
            .iter()
            .fold(0.0_f32, |max, &length| max.max(length))
            + text_padding;
        let groups = (0..len)
            .map(|index| {
                let line = SceneLineMark {
                    len: 2,
                    x: vec![0.0, line_lengths[index]].into(),
                    y: vec![0.0, 0.0].into(),
                    stroke_width: stroke_widths[index],
                    // An empty dash array is a solid line.
                    stroke_dash: stroke_dashes[index].clone().filter(|dash| !dash.is_empty()),
                    stroke: strokes[index].clone(),
                    stroke_cap: self.stroke_cap,
                    stroke_join: self.stroke_join,
                    ..Default::default()
                };
                let content = vec![
                    SceneMark::Line(line).with_interactive(false),
                    make_entry_label(&texts[index], text_x, 0.0, style),
                ];
                let bounds = SceneGroup {
                    marks: content.clone(),
                    ..Default::default()
                }
                .bounding_box(engine);
                let hit = make_hit_rect(
                    bounds.lower()[0],
                    bounds.lower()[1],
                    bounds.upper()[0],
                    bounds.upper()[1],
                );
                SceneGroup {
                    marks: std::iter::once(hit).chain(content).collect(),
                    ..Default::default()
                }
            })
            .collect();
        (row_height, groups)
    }
}

#[cfg(test)]
mod tests {
    use avenger_common::types::{FontWeight, TextSyntaxMode};
    use avenger_typst_label::bundled_label_engine;

    use super::*;
    use crate::legend::{GuideLegendOutput, Legend, LegendConfig, LegendSwatches};

    /// A red and blue line legend in one column, long enough for every entry.
    fn line_legend(text: ScalarOrArray<String>, style: LegendStyle) -> GuideLegendOutput {
        Legend::new(
            &LegendConfig {
                text,
                swatches: LegendSwatches::Lines(LineSwatches {
                    stroke: ScalarOrArray::new_array(vec![
                        ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                        ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
                    ]),
                    ..Default::default()
                }),
                style,
                ..Default::default()
            },
            &bundled_label_engine(),
        )
        .expect("line legend renders")
        .marks(1_000.0)
    }

    #[test]
    fn itemized_line_legend_reports_interactive_hit_rects() {
        let output = line_legend(
            ScalarOrArray::new_array(vec!["A".to_string(), "B".to_string()]),
            LegendStyle::default(),
        );

        assert_eq!(output.items.len(), 2);
        assert_eq!(output.items[0].group_path, vec![1]);
        assert_eq!(output.items[0].hit_rect_path, vec![1, 0]);
        assert_eq!(output.group.marks[0].interactive(), Some(false));

        let SceneMark::Group(item_group) = &output.group.marks[1] else {
            panic!("legend item should be a group");
        };
        assert_eq!(item_group.marks[0].interactive(), None);
        assert_eq!(item_group.marks[1].interactive(), Some(false));
        assert_eq!(item_group.marks[2].interactive(), Some(false));
        let SceneMark::Rect(hit_rect) = &item_group.marks[0] else {
            panic!("first item mark should be hit rect");
        };
        assert!(hit_rect.width.is_some());
        assert!(hit_rect.height.is_some());
    }

    #[test]
    fn line_legend_row_advance_uses_label_syntax_mode() {
        // Display math is taller than the font size, unlike its source as plain text.
        let labels = vec![
            "$display(sum_(i=1)^n x_i)$".to_string(),
            "$J_0(x)$".to_string(),
        ];
        let output = line_legend(
            ScalarOrArray::new_array(labels.clone()),
            LegendStyle {
                label_syntax: TextSyntaxMode::TypstMarkup,
                ..Default::default()
            },
        );

        let SceneMark::Group(first_group) = &output.group.marks[1] else {
            panic!("first legend item should be a group");
        };
        let SceneMark::Group(second_group) = &output.group.marks[2] else {
            panic!("second legend item should be a group");
        };
        let actual_row_advance = second_group.origin[1] - first_group.origin[1];

        let typst_text_bbox = SceneTextMark {
            text: labels.clone().into(),
            font: "sans-serif".to_string().into(),
            font_size: LegendStyle::default().label_font_size.into(),
            font_weight: FontWeight(300).into(),
            text_syntax: TextSyntaxMode::TypstMarkup,
            ..Default::default()
        }
        .bounding_box(&bundled_label_engine());
        let plain_text_bbox = SceneTextMark {
            text: labels.into(),
            font: "sans-serif".to_string().into(),
            font_size: LegendStyle::default().label_font_size.into(),
            font_weight: FontWeight(300).into(),
            text_syntax: TextSyntaxMode::Plain,
            ..Default::default()
        }
        .bounding_box(&bundled_label_engine());

        assert!(typst_text_bbox.height() > plain_text_bbox.height() + 1.0);
        assert!(
            actual_row_advance >= typst_text_bbox.height().floor(),
            "row advance should use Typst label height, got {actual_row_advance} for {:?}",
            typst_text_bbox.height()
        );
    }
}
