use std::sync::Arc;

use crate::{error::AvengerGuidesError, legend::compute_encoding_length};
use avenger_common::types::{StrokeCap, StrokeJoin};
use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark, text::SceneTextMark};
use avenger_text::types::{FontWeight, TextAlign, TextBaseline};

/// Symbol legends
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
            font_family: "Atkinson Hyperlegible Next".into(),
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
        }
    }
}

pub fn make_line_legend(config: &LineLegendConfig) -> Result<SceneGroup, AvengerGuidesError> {
    // Compute the common encoding length
    let len = compute_encoding_length(&[
        config.text.len(),
        config.stroke.len(),
        config.stroke_width.len(),
        config.stroke_dash.len(),
    ])?;

    let mut groups: Vec<SceneMark> = Vec::with_capacity(len + 1);

    let text_strs = config.text.as_vec(len, None);

    let all_text_mark = SceneTextMark {
        text: text_strs.into(),
        font: config.font_family.clone(),
        font_size: config.font_size.clone(),
        x: 0.0.into(),
        y: 0.0.into(),
        ..Default::default()
    };
    let all_text_bbox = all_text_mark.bounding_box();
    let _max_text_width = all_text_bbox.width();
    let max_text_height = all_text_bbox.height();
    let legend_group_height = max_text_height;

    // Always use consistent padding for layout stability
    let bg_padding = config.background_padding.unwrap_or(max_text_height / 2.0);

    // Position legend content with padding from the background rect origin
    let mut line_group_y = bg_padding + legend_group_height / 2.0;

    // Add title if present
    let title_height = if let Some(ref title_text) = config.title {
        // Use same font size as legend items for consistency
        let legend_font_size = 10.0;
        let title_mark = SceneTextMark {
            text: title_text.clone().into(),
            x: bg_padding.into(),
            y: (bg_padding + legend_font_size / 2.0).into(), // Center title vertically in its space
            font_size: 12.0.into(),                          // Legend title size
            font_weight: FontWeight::Number(400.0).into(),   // Medium weight for titles
            font: config.font_family.as_vec(1, None)[0].clone().into(),
            color: ColorOrGradient::Color([0.173, 0.173, 0.173, 1.0]).into(), // #2C2C2C
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            ..Default::default()
        };
        groups.push(SceneMark::Text(Arc::new(title_mark)));

        let title_space = legend_font_size + 4.0;
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

    // Find the maximum line length for text alignment
    let max_line_length = line_lengths.iter().fold(0.0f32, |max, &len| max.max(len));

    for i in 0..len {
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
        );
        groups.push(SceneMark::Group(group));
        line_group_y += legend_group_height;
    }

    // Measure the content bounds
    let temp_group = SceneGroup {
        marks: groups.clone(),
        ..Default::default()
    };
    let content_bbox = temp_group.bounding_box();

    // Calculate total dimensions including padding
    // The background rect always exists and defines our coordinate system
    // Add symmetric padding on all sides
    let bg_width = content_bbox.width() + bg_padding * 2.0;
    let bg_height = legend_group_height * len as f32 + bg_padding * 2.0 + title_height;

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
        ..Default::default()
    };

    // Insert background rect first, then legend content
    let mut final_marks = vec![SceneMark::Rect(bg)];
    final_marks.extend(groups);

    Ok(SceneGroup {
        marks: final_marks,
        clip: avenger_scenegraph::marks::group::Clip::None,
        ..Default::default()
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
) -> SceneGroup {
    // Line and text should be positioned relative to the group's local origin
    let x0 = 0.0;
    let x1 = line_length;
    // Text position is based on max_line_length to ensure horizontal alignment
    let text_x = max_line_length + text_padding;

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
        text: text.to_string().into(),
        x: text_x.into(),
        y: 0.0.into(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Middle.into(),
        font_size: 11.0.into(),                        // Legend item size
        font_weight: FontWeight::Number(300.0).into(), // Regular weight for legend items
        color: ColorOrGradient::Color([0.235, 0.235, 0.235, 1.0]).into(), // #3C3C3C
        ..Default::default()
    };

    SceneGroup {
        origin: [x_offset, y],
        marks: vec![
            SceneMark::Line(single_line_mark),
            SceneMark::Text(Arc::new(text_mark)),
        ],
        ..Default::default()
    }
}
