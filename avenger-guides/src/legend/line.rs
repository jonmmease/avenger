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

use super::{
    compute_encoding_length, make_background, make_entry_label, make_hit_rect, make_title,
    GuideLegendItem, GuideLegendOutput, LegendStyle,
};
use crate::error::AvengerGuidesError;

/// Line legends
#[derive(Debug, Clone)]
pub struct LineLegendConfig {
    pub title: Option<String>,
    pub text: ScalarOrArray<String>,
    pub stroke_width: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_dash: ScalarOrArray<Option<Vec<f32>>>,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,

    /// Padding between the line segment and the text
    pub text_padding: f32,

    /// Length of the line in the legend (can be different for each entry)
    pub line_length: ScalarOrArray<f32>,

    pub style: LegendStyle,
}

impl Default for LineLegendConfig {
    fn default() -> Self {
        Self {
            title: None,
            text: ScalarOrArray::new_scalar("".to_string()),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke_dash: None.into(),
            stroke_width: 2.0.into(),
            stroke_cap: StrokeCap::Butt,
            stroke_join: StrokeJoin::Miter,
            text_padding: 2.0,
            line_length: ScalarOrArray::new_scalar(10.0),
            style: LegendStyle::default(),
        }
    }
}

/// One entry's line and label.
struct LineEntry<'a> {
    text: &'a str,
    stroke_width: f32,
    stroke_dash: Option<Vec<f32>>,
    stroke: ColorOrGradient,
    line_length: f32,
}

/// A line legend at the group origin, with a row for each entry and the scene paths that
/// interactions can target.
pub fn make_line_legend(
    config: &LineLegendConfig,
    text_engine: &LabelEngine,
) -> Result<GuideLegendOutput, AvengerGuidesError> {
    // Compute the common encoding length
    let len = compute_encoding_length(&[
        config.text.len(),
        config.stroke.len(),
        config.stroke_width.len(),
        config.stroke_dash.len(),
    ])?;
    let style = &config.style;
    let texts = config.text.as_vec(len, None);

    // Every row is as tall as the tallest label.
    let row_height = SceneTextMark {
        text: texts.clone().into(),
        font: style.label_font_family.clone().into(),
        font_size: style.label_font_size.into(),
        font_weight: style.label_font_weight.into(),
        text_syntax: style.label_syntax,
        ..Default::default()
    }
    .bounding_box(text_engine)
    .height();

    let padding = style.background_padding;
    let mut marks = Vec::with_capacity(len + 1);
    let mut top = padding;
    if let Some(title) = &config.title {
        let (title, height) = make_title(title, style, text_engine)?;
        marks.push(title);
        top += height;
    }

    let stroke_widths = config.stroke_width.as_vec(len, None);
    let stroke_dashes = config.stroke_dash.as_vec(len, None);
    let strokes = config.stroke.as_vec(len, None);
    let line_lengths = config.line_length.as_vec(len, None);
    // Labels start after the longest line, so that they align.
    let text_x = line_lengths
        .iter()
        .fold(0.0_f32, |max, &length| max.max(length))
        + config.text_padding;

    let mut items = Vec::with_capacity(len);
    let mut y = (top + row_height / 2.0).round();
    for index in 0..len {
        let entry = LineEntry {
            text: &texts[index],
            stroke_width: stroke_widths[index],
            // An empty dash array is a solid line.
            stroke_dash: stroke_dashes[index].clone().filter(|dash| !dash.is_empty()),
            stroke: strokes[index].clone(),
            line_length: line_lengths[index],
        };
        let group = make_line_group(entry, [padding, y], text_x, config, text_engine);
        // The background comes first, so entry paths start at 1.
        let group_path = vec![marks.len() + 1];
        items.push(GuideLegendItem {
            index,
            label: texts[index].clone(),
            hit_rect_path: vec![group_path[0], 0],
            group_path,
        });
        marks.push(SceneMark::Group(group));
        y = (y + row_height).round();
    }

    let content = SceneGroup {
        marks: marks.clone(),
        ..Default::default()
    }
    .bounding_box(text_engine);
    let background = make_background(
        style,
        content.width() + 2.0 * padding,
        top + row_height * len as f32 + padding,
    );
    Ok(GuideLegendOutput {
        group: SceneGroup {
            marks: std::iter::once(background).chain(marks).collect(),
            ..Default::default()
        },
        items,
    })
}

/// An entry's row: its line from the origin, its label at `text_x`, and a hit rect over both.
fn make_line_group(
    entry: LineEntry,
    origin: [f32; 2],
    text_x: f32,
    config: &LineLegendConfig,
    text_engine: &LabelEngine,
) -> SceneGroup {
    let line = SceneLineMark {
        len: 2,
        x: vec![0.0, entry.line_length].into(),
        y: vec![0.0, 0.0].into(),
        stroke_width: entry.stroke_width,
        stroke_dash: entry.stroke_dash,
        stroke: entry.stroke,
        stroke_cap: config.stroke_cap,
        stroke_join: config.stroke_join,
        ..Default::default()
    };
    let content = vec![
        SceneMark::Line(line).with_interactive(false),
        make_entry_label(entry.text, text_x, 0.0, &config.style),
    ];
    let bounds = SceneGroup {
        marks: content.clone(),
        ..Default::default()
    }
    .bounding_box(text_engine);
    let hit = make_hit_rect(
        bounds.lower()[0],
        bounds.lower()[1],
        bounds.upper()[0],
        bounds.upper()[1],
    );
    SceneGroup {
        origin,
        marks: std::iter::once(hit).chain(content).collect(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use avenger_common::types::{FontWeight, TextSyntaxMode};
    use avenger_typst_label::bundled_label_engine;

    use super::*;

    #[test]
    fn itemized_line_legend_reports_interactive_hit_rects() {
        let output = make_line_legend(
            &LineLegendConfig {
                text: ScalarOrArray::new_array(vec!["A".to_string(), "B".to_string()]),
                stroke: ScalarOrArray::new_array(vec![
                    ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                    ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
                ]),
                ..Default::default()
            },
            &avenger_typst_label::bundled_label_engine(),
        )
        .expect("line legend renders");

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
        let output = make_line_legend(
            &LineLegendConfig {
                text: ScalarOrArray::new_array(labels.clone()),
                stroke: ScalarOrArray::new_array(vec![
                    ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                    ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
                ]),
                style: LegendStyle {
                    label_syntax: TextSyntaxMode::TypstMarkup,
                    ..Default::default()
                },
                ..Default::default()
            },
            &avenger_typst_label::bundled_label_engine(),
        )
        .expect("line legend renders");

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
