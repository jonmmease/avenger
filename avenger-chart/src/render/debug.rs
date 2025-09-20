//! Debug utilities for visualizing layout bounds

use avenger_common::types::ColorOrGradient;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use std::sync::Arc;

/// Create debug rectangles to visualize Taffy layout bounds
pub fn create_debug_layout_rects(layout: &crate::layout::LayoutResult) -> Vec<SceneMark> {
    let mut debug_marks = Vec::new();

    // Plot area - magenta outline
    let plot_rect = SceneRectMark {
        x: layout.plot_area.x.into(),
        y: layout.plot_area.y.into(),
        width: Some(layout.plot_area.width.into()),
        height: Some(layout.plot_area.height.into()),
        fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(), // Transparent
        stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(), // Magenta with 0.7 opacity
        stroke_width: 1.0.into(),
        zindex: Some(20),
        ..Default::default()
    };
    debug_marks.push(SceneMark::Rect(plot_rect));

    // Plot area label
    let plot_label = SceneTextMark {
        text: "plot-area".into(),
        x: (layout.plot_area.x + 2.0).into(),
        y: (layout.plot_area.y + 10.0).into(),
        font_size: 8.0.into(),
        color: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(),
        zindex: Some(20),
        ..Default::default()
    };
    debug_marks.push(SceneMark::Text(Arc::new(plot_label)));

    // Axes - magenta outlines with labels
    for (position, bounds) in &layout.axes {
        let axis_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(),
            stroke_width: 1.0.into(),
            zindex: Some(20),
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(axis_rect));

        // Add axis label - check if it's an overflow pseudo-axis
        let (axis_label, label_x, label_y, angle, align, baseline) = match position {
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
                -90.0,
                avenger_text::types::TextAlign::Right,
                avenger_text::types::TextBaseline::Bottom,
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
        };

        let label = SceneTextMark {
            text: axis_label.into(),
            x: label_x.into(),
            y: label_y.into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(),
            angle: angle.into(),
            align: align.into(),
            baseline: baseline.into(),
            zindex: Some(20),
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(label)));
    }

    // Legends - magenta outlines
    for (channel, bounds) in &layout.legends {
        let legend_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(),
            stroke_width: 1.0.into(),
            zindex: Some(20),
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(legend_rect));

        // Add label for legend channel
        let label = SceneTextMark {
            text: channel.clone().into(),
            x: (bounds.x + 2.0).into(),
            y: (bounds.y + 10.0).into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(),
            zindex: Some(20),
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(label)));
    }

    // Title - magenta outline
    if let Some(bounds) = &layout.title {
        let title_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(),
            stroke_width: 1.0.into(),
            zindex: Some(20),
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(title_rect));

        // Add title label - right aligned to avoid overlapping with text
        let title_label = SceneTextMark {
            text: "title".into(),
            x: (bounds.x + bounds.width - 5.0).into(),
            y: (bounds.y + 10.0).into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(),
            align: avenger_text::types::TextAlign::Right.into(),
            zindex: Some(20),
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(title_label)));
    }

    // Subtitle - magenta outline
    if let Some(bounds) = &layout.subtitle {
        let subtitle_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(),
            stroke_width: 1.0.into(),
            zindex: Some(20),
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(subtitle_rect));

        // Add subtitle label - right aligned to avoid overlapping with text
        let subtitle_label = SceneTextMark {
            text: "subtitle".into(),
            x: (bounds.x + bounds.width - 5.0).into(),
            y: (bounds.y + 10.0).into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.7]).into(),
            align: avenger_text::types::TextAlign::Right.into(),
            zindex: Some(20),
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(subtitle_label)));
    }

    debug_marks
}
