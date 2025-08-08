use std::sync::Arc;

use crate::{error::AvengerGuidesError, legend::compute_encoding_length};
use avenger_common::types::StrokeCap;
use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark, text::SceneTextMark};
use avenger_text::types::{TextAlign, TextBaseline};

/// Symbol legends
pub struct LineLegendConfig {
    pub title: Option<String>,
    pub text: ScalarOrArray<String>,
    pub stroke_width: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_dash: ScalarOrArray<Option<Vec<f32>>>,
    pub stroke_cap: StrokeCap,

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

    /// Padding between the symbol and the text
    pub text_padding: f32,

    /// Length of the line in the legend
    pub line_length: f32,
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
            font_size: 10.0.into(),
            font_family: "sans-serif".into(),
            inner_width: 100.0,
            inner_height: 100.0,
            outer_margin: 4.0,
            text_padding: 2.0,
            entry_margin: 2.0,
            line_length: 10.0,
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

    let mut groups: Vec<SceneMark> = Vec::with_capacity(len);

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
    let legend_group_height = max_text_height + config.entry_margin * 2.0;

    let mut y = 0.0;
    let center_y = y + legend_group_height / 2.0;

    // Expand encodings
    let text_strs = config.text.as_vec(len, None);
    let stroke_widths = config.stroke_width.as_vec(len, None);
    let stroke_dashes = config.stroke_dash.as_vec(len, None);
    let stroke_colors = config.stroke.as_vec(len, None);

    for i in 0..len {
        let group = make_line_group(
            y,
            center_y,
            &text_strs[i],
            stroke_widths[i],
            &stroke_dashes[i],
            &stroke_colors[i],
            config.stroke_cap,
            config,
        );
        // let group_rtree = MarkRTree::from_scene_group(&group);

        groups.push(SceneMark::Group(group));
        y += legend_group_height;
    }

    // Measure the overall bounds to create a clip rect
    let temp_group = SceneGroup {
        marks: groups.clone(),
        ..Default::default()
    };
    let bbox = temp_group.bounding_box();
    // The bounding box should include all content
    // Add padding all around to prevent clipping (matches symbol legend)
    let padding = 4.0; // keep local to avoid cross-crate cycle
    let width = bbox.width() + 2.0 * padding;
    let height = bbox.height() + 2.0 * padding;

    Ok(SceneGroup {
        marks: groups,
        clip: avenger_scenegraph::marks::group::Clip::Rect {
            x: -padding,
            y: -padding,
            width,
            height,
        },
        ..Default::default()
    })
}

#[allow(clippy::too_many_arguments)]
fn make_line_group(
    y: f32,
    center_y: f32,
    text: &str,
    stroke_width: f32,
    stroke_dash: &Option<Vec<f32>>,
    stroke_color: &ColorOrGradient,
    stroke_cap: StrokeCap,
    config: &LineLegendConfig,
) -> SceneGroup {
    //
    let x0 = 0.0;
    let x1 = config.line_length;

    let text_x = x1 + config.text_padding;

    // Line
    let single_line_mark = SceneLineMark {
        len: 2,
        x: vec![x0, x1].into(),
        y: vec![center_y, center_y].into(),
        stroke_width,
        stroke_dash: stroke_dash.clone(),
        stroke: stroke_color.clone(),
        stroke_cap,
        ..Default::default()
    };

    // Text
    let text_mark = SceneTextMark {
        text: text.to_string().into(),
        x: text_x.into(),
        y: center_y.into(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Middle.into(),
        font_size: 10.0.into(),
        ..Default::default()
    };

    // Legend entry group

    SceneGroup {
        origin: [config.inner_width + config.outer_margin, y],
        marks: vec![
            SceneMark::Line(single_line_mark),
            SceneMark::Text(Arc::new(text_mark)),
        ],
        ..Default::default()
    }
}
