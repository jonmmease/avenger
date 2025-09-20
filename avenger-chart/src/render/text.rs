//! Text rendering utilities for titles and subtitles

use crate::error::AvengerChartError;
use crate::layout::LayoutBounds;
use crate::plot::{Plot, PlotSubtitle, PlotTitle};
use crate::render::Padding;
use crate::theme::Theme;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_text::types::{TextAlign, TextBaseline};
use std::sync::Arc;

/// Create title mark if configured
pub fn create_title<T: Theme>(
    title: &PlotTitle,
    _total_width: f32,
    _padding: &Padding,
    layout_bounds: Option<LayoutBounds>,
    _plot_area: Option<LayoutBounds>,
    theme: &T,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    use crate::theme::context::ThemeContext;

    // If we have layout bounds from Taffy, place the title left-aligned within its bounds
    let (x, y) = if let Some(bounds) = layout_bounds {
        // Use the title node's x position, not the plot area's
        (bounds.x, bounds.y + bounds.height / 2.0)
    } else {
        // Fallback positioning if no layout bounds
        (10.0, 20.0)
    };

    // Create theme context for title
    let context = ThemeContext::new()
        .with_element("title")
        .with_parent_element("chart");

    let text_mark = SceneTextMark {
        text: title.text.clone().into(),
        x: x.into(),
        y: y.into(),
        color: theme.color(&context).into(),
        font_size: theme.font_size(&context).into(),
        font_family: theme.font(&context).into(),
        font_style: theme.font_style(&context).into(),
        font_weight: theme.font_weight(&context).into(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Middle.into(),
        ..Default::default()
    };

    Ok(vec![SceneMark::Text(Arc::new(text_mark))])
}

/// Create subtitle mark if configured
pub fn create_subtitle<T: Theme>(
    subtitle: &PlotSubtitle,
    _total_width: f32,
    _padding: &Padding,
    layout_bounds: Option<LayoutBounds>,
    _plot_area: Option<LayoutBounds>,
    theme: &T,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    use crate::theme::context::ThemeContext;

    // If we have layout bounds from Taffy, place the subtitle left-aligned within its bounds
    let (x, y) = if let Some(bounds) = layout_bounds {
        // Use the subtitle node's x position, not the plot area's
        (bounds.x, bounds.y + bounds.height / 2.0)
    } else {
        // Fallback positioning if no layout bounds
        (10.0, 40.0)
    };

    // Create theme context for subtitle
    let context = ThemeContext::new()
        .with_element("subtitle")
        .with_parent_element("chart");

    let text_mark = SceneTextMark {
        text: subtitle.text.clone().into(),
        x: x.into(),
        y: y.into(),
        color: theme.color(&context).into(),
        font_size: theme.font_size(&context).into(),
        font_family: theme.font(&context).into(),
        font_style: theme.font_style(&context).into(),
        font_weight: theme.font_weight(&context).into(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Middle.into(),
        ..Default::default()
    };

    Ok(vec![SceneMark::Text(Arc::new(text_mark))])
}