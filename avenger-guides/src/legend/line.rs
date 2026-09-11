use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scenegraph::marks::{
    group::SceneGroup, line::SceneLineMark, mark::SceneMark, rect::SceneRectMark,
    text::SceneTextMark,
};
use avenger_text::{
    default_text_engine,
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode},
    LabelParams,
};

use crate::{
    error::AvengerGuidesError,
    legend::{compute_encoding_length, GuideLegendItem, GuideLegendOutput},
};

/// Symbol legends
#[derive(Debug)]
pub struct LineLegendConfig {
    pub title: Option<String>,
    pub text: ScalarOrArray<String>,
    pub stroke_width: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_dash: ScalarOrArray<Option<Vec<f32>>>,
    pub stroke_cap: StrokeCap,
    pub stroke_join: Option<StrokeJoin>,

    pub font_size: ScalarOrArray<f32>,
    pub font_family: ScalarOrArray<String>,

    /// Width of the chart area that the legend may be placed next to
    pub inner_width: f32,

    /// Height of the chart area that the legend may be placed next to
    pub inner_height: f32,

    /// Margin around the legend, separating it from the chart area
    pub outer_margin: f32,

    /// Margin around the legend, separating it from the chart area
    pub entry_margin: f32,

    /// Padding between the line segment and the text
    pub text_padding: f32,

    /// Length of the line in the legend (can be different for each entry)
    pub line_length: ScalarOrArray<f32>,

    /// Background rect styling
    pub background_fill: Option<ColorOrGradient>,
    pub background_stroke: Option<ColorOrGradient>,
    pub background_corner_radius: Option<f32>,
    pub background_padding: Option<f32>,

    /// Text colors
    pub title_color: Option<[f32; 4]>,
    pub label_color: Option<[f32; 4]>,

    /// Typography configuration for title
    pub title_font_family: Option<String>,
    pub title_font_size: Option<f32>,
    pub title_font_weight: Option<FontWeight>,
    pub title_syntax_mode: TextSyntaxMode,
    pub title_text_params: LabelParams,

    /// Typography configuration for labels
    pub label_font_family: Option<String>,
    pub label_font_size: Option<f32>,
    pub label_font_weight: Option<FontWeight>,
    pub label_syntax_mode: TextSyntaxMode,
    pub label_text_params: LabelParams,
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
            stroke_join: Some(StrokeJoin::Miter),
            font_size: 10.0.into(),
            font_family: "sans-serif".into(),
            inner_width: 100.0,
            inner_height: 100.0,
            outer_margin: 4.0,
            text_padding: 2.0,
            entry_margin: 2.0,
            line_length: ScalarOrArray::new_scalar(10.0),
            background_fill: None,
            background_stroke: None,
            background_corner_radius: None,
            background_padding: None,
            title_color: None,
            label_color: None,
            title_font_family: None,
            title_font_size: None,
            title_font_weight: None,
            title_syntax_mode: TextSyntaxMode::Plain,
            title_text_params: LabelParams::default(),
            label_font_family: None,
            label_font_size: None,
            label_font_weight: None,
            label_syntax_mode: TextSyntaxMode::Plain,
            label_text_params: LabelParams::default(),
        }
    }
}

pub fn make_line_legend(config: &LineLegendConfig) -> Result<SceneGroup, AvengerGuidesError> {
    Ok(make_line_legend_itemized(config)?.group)
}

pub fn make_line_legend_with_text_engine(
    config: &LineLegendConfig,
    text_engine: &avenger_text::TextEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    Ok(make_line_legend_itemized_with_text_engine(config, text_engine)?.group)
}

pub fn make_line_legend_itemized(
    config: &LineLegendConfig,
) -> Result<GuideLegendOutput, AvengerGuidesError> {
    make_line_legend_itemized_with_text_engine(config, &default_text_engine())
}

pub fn make_line_legend_itemized_with_text_engine(
    config: &LineLegendConfig,
    text_engine: &avenger_text::TextEngine,
) -> Result<GuideLegendOutput, AvengerGuidesError> {
    // Compute the common encoding length
    let len = compute_encoding_length(&[
        config.text.len(),
        config.stroke.len(),
        config.stroke_width.len(),
        config.stroke_dash.len(),
    ])?;

    let mut groups: Vec<SceneMark> = Vec::with_capacity(len + 1);

    let text_strs = config.text.as_vec(len, None);

    // Use the label font configuration for measuring text (same as rendering)
    let measure_font =
        config
            .label_font_family
            .clone()
            .unwrap_or_else(|| match config.font_family.value() {
                avenger_common::value::ScalarOrArrayValue::Scalar(s) => s.clone(),
                _ => "sans-serif".to_string(),
            });
    let measure_font_size =
        config
            .label_font_size
            .unwrap_or_else(|| match config.font_size.value() {
                avenger_common::value::ScalarOrArrayValue::Scalar(s) => *s,
                _ => 11.0,
            });

    // Also use the same font weight for measuring as for rendering
    let measure_font_weight = config
        .label_font_weight
        .unwrap_or(FontWeight::Number(300.0));

    let all_text_mark = SceneTextMark {
        text: text_strs.into(),
        font: measure_font.clone().into(),
        font_size: measure_font_size.into(),
        font_weight: measure_font_weight.into(),
        x: 0.0.into(),
        y: 0.0.into(),
        text_syntax: config.label_syntax_mode,
        text_params: config.label_text_params.clone(),
        ..Default::default()
    };
    let all_text_bbox = all_text_mark.bounding_box_with_text_engine(text_engine);
    let _max_text_width = all_text_bbox.width();
    let max_text_height = all_text_bbox.height();
    let legend_group_height = max_text_height;

    // Always use consistent padding for layout stability
    let bg_padding = config.background_padding.unwrap_or(max_text_height / 2.0);

    // Position legend content with padding from the background rect origin
    // Round to pixel boundaries for crisp rendering
    let mut line_group_y = (bg_padding + legend_group_height / 2.0).round();

    // Add title if present
    let title_height = if let Some(ref title_text) = config.title {
        // Use configurable or default font size
        let title_font_size = config.title_font_size.unwrap_or(12.0);
        let title_font = config
            .title_font_family
            .clone()
            .unwrap_or_else(|| "sans-serif".to_string());
        let title_font_weight = config
            .title_font_weight
            .unwrap_or(FontWeight::Number(400.0));
        let title_bounds = text_engine.measure_bounds(&TextMeasurementConfig {
            text: title_text,
            font: &title_font,
            font_size: title_font_size,
            font_weight: title_font_weight,
            font_style: FontStyle::Normal,
            syntax_mode: config.title_syntax_mode,
            params: &config.title_text_params,
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        })?;

        let title_mark = SceneTextMark {
            clip: false,
            text: title_text.clone().into(),
            x: bg_padding.into(),
            y: (bg_padding + title_font_size / 2.0).into(), // Center title vertically in its space
            font_size: title_font_size.into(),
            font_weight: title_font_weight.into(),
            font: title_font.into(),
            color: ColorOrGradient::Color(config.title_color.unwrap_or([0.173, 0.173, 0.173, 1.0]))
                .into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            text_syntax: config.title_syntax_mode,
            text_params: config.title_text_params.clone(),
            ..Default::default()
        };
        groups.push(SceneMark::Text(Arc::new(title_mark)).with_interactive(false));

        let title_space = title_bounds.line_height + 4.0;
        line_group_y += title_space;
        title_space
    } else {
        0.0
    };

    // Expand encodings
    let text_strs = config.text.as_vec(len, None);
    let stroke_widths = config.stroke_width.as_vec(len, None);
    let stroke_dashes = config.stroke_dash.as_vec(len, None);
    let stroke_colors = config.stroke.as_vec(len, None);
    let line_lengths = config.line_length.as_vec(len, None);
    let mut items = Vec::with_capacity(len);

    // Find the maximum line length for text alignment
    let max_line_length = line_lengths.iter().fold(0.0f32, |max, &len| max.max(len));

    for i in 0..len {
        let group_path = vec![groups.len() + 1];
        let group = make_line_group(
            line_group_y,
            bg_padding,
            &text_strs[i],
            stroke_widths[i],
            &stroke_dashes[i],
            &stroke_colors[i],
            config.stroke_cap,
            config.stroke_join,
            line_lengths[i],
            max_line_length,
            config.text_padding,
            config.label_color,
            Some(measure_font.as_str()),
            Some(measure_font_size),
            Some(&measure_font_weight),
            config.label_syntax_mode,
            &config.label_text_params,
            text_engine,
        )?;
        groups.push(SceneMark::Group(group));
        items.push(GuideLegendItem {
            index: i,
            label: text_strs[i].clone(),
            hit_rect_path: vec![group_path[0], 0],
            group_path,
        });
        // Advance by group height (no additional per-entry spacing by default)
        line_group_y = (line_group_y + legend_group_height).round();
    }

    // Measure the content bounds
    let temp_group = SceneGroup {
        marks: groups.clone(),
        ..Default::default()
    };
    let content_bbox = temp_group.bounding_box_with_text_engine(text_engine);

    // Calculate total dimensions including padding
    // The background rect always exists and defines our coordinate system
    // Add symmetric padding on all sides and round to pixel boundaries
    let bg_width = (content_bbox.width() + bg_padding * 2.0).round();
    // Total height: rows + symmetric padding + title height
    let bg_height = (legend_group_height * len as f32 + bg_padding * 2.0 + title_height).round();

    // Create a background rect at origin (0, 0)
    // This provides consistent layout whether visible or not
    let bg = SceneRectMark {
        x: 0.0.into(),
        y: 0.0.into(),
        width: Some(bg_width.into()),
        height: Some(bg_height.into()),
        fill: config
            .background_fill
            .clone()
            .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])) // Transparent by default
            .into(),
        stroke: config
            .background_stroke
            .clone()
            .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])) // No stroke by default
            .into(),
        stroke_width: if config.background_stroke.is_some() {
            1.0.into()
        } else {
            0.0.into()
        },
        corner_radius: config.background_corner_radius.unwrap_or(0.0).into(),
        zindex: Some(0), // Background should be behind content
        interactive: false,
        ..Default::default()
    };

    // Insert background rect first, then legend content
    let mut final_marks = vec![SceneMark::Rect(bg)];
    final_marks.extend(groups);

    Ok(GuideLegendOutput {
        group: SceneGroup {
            marks: final_marks,
            clip: avenger_scenegraph::marks::group::Clip::None,
            ..Default::default()
        },
        items,
        continuous_surfaces: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn make_line_group(
    y: f32,
    x_offset: f32,
    text: &str,
    stroke_width: f32,
    stroke_dash: &Option<Vec<f32>>,
    stroke_color: &ColorOrGradient,
    stroke_cap: StrokeCap,
    stroke_join: Option<StrokeJoin>,
    line_length: f32,
    max_line_length: f32,
    text_padding: f32,
    label_color: Option<[f32; 4]>,
    label_font_family: Option<&str>,
    label_font_size: Option<f32>,
    label_font_weight: Option<&FontWeight>,
    label_syntax_mode: TextSyntaxMode,
    label_text_params: &LabelParams,
    text_engine: &avenger_text::TextEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    // Line and text should be positioned relative to the group's local origin
    let x0 = 0.0;
    let x1 = line_length;
    // Text position is based on max_line_length to ensure horizontal alignment
    let text_x = (max_line_length + text_padding).round();

    // Line
    // Convert empty dash array to None (solid line)
    let stroke_dash_normalized =
        stroke_dash
            .as_ref()
            .and_then(|d| if d.is_empty() { None } else { Some(d.clone()) });

    let single_line_mark = SceneLineMark {
        len: 2,
        x: vec![x0, x1].into(),
        y: vec![0.0, 0.0].into(),
        stroke_width,
        stroke_dash: stroke_dash_normalized,
        stroke: stroke_color.clone(),
        stroke_cap,
        stroke_join: stroke_join.unwrap_or(StrokeJoin::Miter),
        ..Default::default()
    };

    tracing::debug!(
        len = single_line_mark.len,
        x = ?[x0, x1],
        y = ?[0.0, 0.0],
        stroke_width = stroke_width,
        stroke_dash = ?single_line_mark.stroke_dash,
        stroke = ?stroke_color,
        "Creating SceneLineMark"
    );

    // Text
    let text_mark = SceneTextMark {
        clip: false,
        text: text.to_string().into(),
        x: text_x.into(),
        y: 0.0.into(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Middle.into(),
        font_size: label_font_size.unwrap_or(11.0).into(),
        font: label_font_family.unwrap_or("sans-serif").to_string().into(),
        font_weight: label_font_weight
            .cloned()
            .unwrap_or(FontWeight::Number(300.0))
            .into(),
        color: ColorOrGradient::Color(label_color.unwrap_or([0.235, 0.235, 0.235, 1.0])).into(),
        text_syntax: label_syntax_mode,
        text_params: label_text_params.clone(),
        ..Default::default()
    };
    let content_marks = vec![
        SceneMark::Line(single_line_mark).with_interactive(false),
        SceneMark::Text(Arc::new(text_mark)).with_interactive(false),
    ];
    let content_bbox = SceneGroup {
        marks: content_marks.clone(),
        ..Default::default()
    }
    .bounding_box_with_text_engine(text_engine);

    Ok(SceneGroup {
        origin: [x_offset, y],
        marks: std::iter::once(SceneMark::Rect(SceneRectMark {
            x: content_bbox.lower()[0].into(),
            y: content_bbox.lower()[1].into(),
            width: Some(content_bbox.width().into()),
            height: Some(content_bbox.height().into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke_width: 0.0.into(),
            ..Default::default()
        }))
        .chain(content_marks)
        .collect(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn itemized_line_legend_reports_interactive_hit_rects() {
        let output = make_line_legend_itemized(&LineLegendConfig {
            text: ScalarOrArray::new_array(vec!["A".to_string(), "B".to_string()]),
            stroke: ScalarOrArray::new_array(vec![
                ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
            ]),
            ..Default::default()
        })
        .expect("line legend renders");

        assert_eq!(output.items.len(), 2);
        assert_eq!(output.items[0].group_path, vec![1]);
        assert_eq!(output.items[0].hit_rect_path, vec![1, 0]);
        assert!(!output.group.marks[0].interactive());

        let SceneMark::Group(item_group) = &output.group.marks[1] else {
            panic!("legend item should be a group");
        };
        assert!(item_group.marks[0].interactive());
        assert!(!item_group.marks[1].interactive());
        assert!(!item_group.marks[2].interactive());
        let SceneMark::Rect(hit_rect) = &item_group.marks[0] else {
            panic!("first item mark should be hit rect");
        };
        assert!(hit_rect.width.is_some());
        assert!(hit_rect.height.is_some());
    }

    #[test]
    fn line_legend_row_advance_uses_label_syntax_mode() {
        let labels = vec![
            "$y = sqrt(x) / (1 + x^2)$".to_string(),
            "$J_0(x)$".to_string(),
        ];
        let output = make_line_legend_itemized(&LineLegendConfig {
            text: ScalarOrArray::new_array(labels.clone()),
            stroke: ScalarOrArray::new_array(vec![
                ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
            ]),
            label_syntax_mode: TextSyntaxMode::TypstMarkup,
            ..Default::default()
        })
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
            font_size: 10.0.into(),
            font_weight: FontWeight::Number(300.0).into(),
            text_syntax: TextSyntaxMode::TypstMarkup,
            ..Default::default()
        }
        .bounding_box();
        let plain_text_bbox = SceneTextMark {
            text: labels.into(),
            font: "sans-serif".to_string().into(),
            font_size: 10.0.into(),
            font_weight: FontWeight::Number(300.0).into(),
            text_syntax: TextSyntaxMode::Plain,
            ..Default::default()
        }
        .bounding_box();

        assert!(typst_text_bbox.height() > plain_text_bbox.height() + 1.0);
        assert!(
            actual_row_advance >= typst_text_bbox.height().floor(),
            "row advance should use Typst label height, got {actual_row_advance} for {:?}",
            typst_text_bbox.height()
        );
    }

    #[test]
    fn line_legend_forwards_title_and_label_text_params() {
        let mut title_text_params = LabelParams::default();
        title_text_params.insert(
            "series".to_string(),
            avenger_text::LabelParamValue::Str("Revenue".to_string()),
        );

        let mut label_text_params = LabelParams::default();
        label_text_params.insert("first".to_string(), avenger_text::LabelParamValue::Int(1));
        label_text_params.insert("second".to_string(), avenger_text::LabelParamValue::Int(2));

        let output = make_line_legend_itemized(&LineLegendConfig {
            title: Some("#series".to_string()),
            text: ScalarOrArray::new_array(vec!["#first".to_string(), "#second".to_string()]),
            stroke: ScalarOrArray::new_array(vec![
                ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
            ]),
            title_syntax_mode: TextSyntaxMode::TypstMarkup,
            title_text_params: title_text_params.clone(),
            label_syntax_mode: TextSyntaxMode::TypstMarkup,
            label_text_params: label_text_params.clone(),
            ..Default::default()
        })
        .expect("line legend renders");

        let text_marks = collect_text_marks(&output.group.marks);
        let title_mark = text_marks
            .iter()
            .find(|mark| mark.text.as_vec(1, None)[0] == "#series")
            .expect("title text mark");
        assert_eq!(title_mark.text_params, title_text_params);

        for label in ["#first", "#second"] {
            let label_mark = text_marks
                .iter()
                .find(|mark| mark.text.as_vec(1, None)[0] == label)
                .expect("label text mark");
            assert_eq!(label_mark.text_params, label_text_params);
        }
    }

    fn collect_text_marks(marks: &[SceneMark]) -> Vec<&SceneTextMark> {
        let mut text_marks = Vec::new();
        collect_text_marks_into(marks, &mut text_marks);
        text_marks
    }

    fn collect_text_marks_into<'a>(
        marks: &'a [SceneMark],
        text_marks: &mut Vec<&'a SceneTextMark>,
    ) {
        for mark in marks {
            match mark {
                SceneMark::Text(text) => text_marks.push(text),
                SceneMark::Group(group) => collect_text_marks_into(&group.marks, text_marks),
                _ => {}
            }
        }
    }
}
