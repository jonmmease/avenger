//! Debug utilities for visualizing layout bounds

use avenger_common::types::ColorOrGradient;
use avenger_scales::color::parse_color_string;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use std::sync::Arc;

/// Create debug rectangles to visualize Taffy layout bounds
///
/// # Arguments
/// * `layout` - The layout result to visualize
/// * `color` - Optional color string (HSL format like "hsl(15 65% 60%)") for debug marks. Defaults to magenta
/// * `stroke_width` - Optional stroke width. Defaults to 1.0
/// * `zindex` - Optional z-index. Defaults to 20
/// * `flip_label_align` - If true, align labels on opposite side (for subplots to avoid overlap)
pub fn create_debug_layout_rects(
    layout: &crate::layout::LayoutResult,
    color: Option<String>,
    stroke_width: Option<f32>,
    zindex: Option<i32>,
    flip_label_align: bool,
) -> Vec<SceneMark> {
    let mut debug_marks = Vec::new();

    // Parse color string to RGBA, defaulting to magenta
    let debug_color = if let Some(color_str) = color {
        parse_color_string(&color_str).unwrap_or([1.0, 0.0, 1.0, 0.5])
    } else {
        [1.0, 0.0, 1.0, 0.7] // Default magenta
    };
    let stroke = stroke_width.unwrap_or(1.0);
    let z = zindex.unwrap_or(20);

    // Plot area outline
    let plot_rect = SceneRectMark {
        x: layout.plot_area.x.into(),
        y: layout.plot_area.y.into(),
        width: Some(layout.plot_area.width.into()),
        height: Some(layout.plot_area.height.into()),
        fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(), // Transparent
        stroke: ColorOrGradient::Color(debug_color).into(),
        stroke_width: stroke.into(),
        zindex: Some(z),
        clip: false, // Don't clip debug marks
        ..Default::default()
    };
    debug_marks.push(SceneMark::Rect(plot_rect));

    // Plot area label - align right if flip_label_align is true
    let (label_x, label_align) = if flip_label_align {
        (
            layout.plot_area.x + layout.plot_area.width - 2.0,
            avenger_text::types::TextAlign::Right,
        )
    } else {
        (
            layout.plot_area.x + 2.0,
            avenger_text::types::TextAlign::Left,
        )
    };

    let plot_label = SceneTextMark {
        text: "plot-area".into(),
        x: label_x.into(),
        y: (layout.plot_area.y + 10.0).into(),
        font_size: 8.0.into(),
        color: ColorOrGradient::Color(debug_color).into(),
        align: label_align.into(),
        zindex: Some(z),
        clip: false, // Don't clip debug marks
        ..Default::default()
    };
    debug_marks.push(SceneMark::Text(Arc::new(plot_label)));

    // Guide overflows - outlines with labels
    for (position, bounds) in &layout.guide_overflows {
        let overflow_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color(debug_color).into(),
            stroke_width: stroke.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(overflow_rect));

        // Add overflow region label - flip alignment if needed
        let (overflow_label, label_x, label_y, angle, align, baseline): (
            &str,
            f32,
            f32,
            f32,
            _,
            _,
        ) = if flip_label_align {
            match position {
                crate::cartesian::axis::AxisPosition::Left => (
                    "of-left",
                    bounds.x + 2.0,
                    bounds.y + bounds.height - 2.0,
                    -90.0,
                    avenger_text::types::TextAlign::Left,
                    avenger_text::types::TextBaseline::Top,
                ),
                crate::cartesian::axis::AxisPosition::Right => (
                    "of-right",
                    bounds.x + bounds.width - 2.0,
                    bounds.y + bounds.height - 2.0,
                    90.0,
                    avenger_text::types::TextAlign::Right,
                    avenger_text::types::TextBaseline::Top,
                ),
                crate::cartesian::axis::AxisPosition::Top => (
                    "of-top",
                    bounds.x + bounds.width - 2.0, // Right side instead of left
                    bounds.y,
                    0.0,
                    avenger_text::types::TextAlign::Right, // Flipped from Left
                    avenger_text::types::TextBaseline::Top,
                ),
                crate::cartesian::axis::AxisPosition::Bottom => (
                    "of-bottom",
                    bounds.x + 2.0,
                    bounds.y + bounds.height - 2.0,
                    0.0,
                    avenger_text::types::TextAlign::Left,
                    avenger_text::types::TextBaseline::Bottom,
                ),
            }
        } else {
            match position {
                crate::cartesian::axis::AxisPosition::Left => (
                    "of-left",
                    bounds.x + 2.0,
                    bounds.y + 2.0,
                    -90.0,
                    avenger_text::types::TextAlign::Right,
                    avenger_text::types::TextBaseline::Top,
                ),
                crate::cartesian::axis::AxisPosition::Right => (
                    "of-right",
                    bounds.x + bounds.width - 2.0,
                    bounds.y + 2.0,
                    90.0,
                    avenger_text::types::TextAlign::Left,
                    avenger_text::types::TextBaseline::Top,
                ),
                crate::cartesian::axis::AxisPosition::Top => (
                    "of-top",
                    bounds.x + 2.0,
                    bounds.y,
                    0.0,
                    avenger_text::types::TextAlign::Left,
                    avenger_text::types::TextBaseline::Top,
                ),
                crate::cartesian::axis::AxisPosition::Bottom => (
                    "of-bottom",
                    bounds.x + 2.0,
                    bounds.y + bounds.height - 2.0,
                    0.0,
                    avenger_text::types::TextAlign::Left,
                    avenger_text::types::TextBaseline::Bottom,
                ),
            }
        };

        let overflow_label_mark = SceneTextMark {
            text: overflow_label.into(),
            x: label_x.into(),
            y: label_y.into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color(debug_color).into(),
            angle: angle.into(),
            align: align.into(),
            baseline: baseline.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(overflow_label_mark)));
    }

    // Legends - outlines
    for (channel, bounds) in &layout.legends {
        let legend_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color(debug_color).into(),
            stroke_width: stroke.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(legend_rect));

        // Add label for legend channel
        let label = SceneTextMark {
            text: channel.clone().into(),
            x: (bounds.x + 2.0).into(),
            y: (bounds.y + 10.0).into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color(debug_color).into(),
            zindex: Some(20),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(label)));
    }

    // Title - outline
    if let Some(bounds) = &layout.title {
        let title_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color(debug_color).into(),
            stroke_width: stroke.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(title_rect));

        // Add title label - right aligned to avoid overlapping with text
        let title_label = SceneTextMark {
            text: "title".into(),
            x: (bounds.x + bounds.width - 5.0).into(),
            y: (bounds.y + 10.0).into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color(debug_color).into(),
            align: avenger_text::types::TextAlign::Right.into(),
            zindex: Some(20),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(title_label)));
    }

    // Subtitle - outline
    if let Some(bounds) = &layout.subtitle {
        let subtitle_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color(debug_color).into(),
            stroke_width: stroke.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(subtitle_rect));

        // Add subtitle label - right aligned to avoid overlapping with text
        let subtitle_label = SceneTextMark {
            text: "subtitle".into(),
            x: (bounds.x + bounds.width - 5.0).into(),
            y: (bounds.y + 10.0).into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color(debug_color).into(),
            align: avenger_text::types::TextAlign::Right.into(),
            zindex: Some(20),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(subtitle_label)));
    }

    debug_marks
}
