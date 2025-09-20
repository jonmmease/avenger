//! Title and subtitle rendering
//!
//! This module handles:
//! - Creating title marks with proper positioning
//! - Creating subtitle marks with proper positioning
//! - Applying theme settings to text elements

use super::PlotRenderer;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::render::Padding;
use avenger_common::types::ColorOrGradient;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_text::types::{TextAlign, TextBaseline};

impl<C: CoordinateSystem> PlotRenderer<'_, C> {
    /// Create title mark if configured
    pub(super) fn create_title(
        &self,
        _total_width: f32,
        _padding: &Padding,
        layout_bounds: Option<crate::layout::LayoutBounds>,
        _plot_area: Option<crate::layout::LayoutBounds>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(title) = self.plot.get_title() else {
            return Ok(Vec::new());
        };

        // Note: The text module now uses the newer Theme trait implementation,
        // but mod.rs still uses the old theme methods. This is a compatibility issue
        // we'll need to address. For now, using the old implementation inline.
        let theme = self.plot.get_theme();

        let (x, y) = if let Some(bounds) = layout_bounds {
            (bounds.x, bounds.y + bounds.height / 2.0)
        } else {
            (10.0, 10.0)
        };

        let text = SceneTextMark {
            text: title.text.clone().into(),
            x: x.into(),
            y: y.into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            font: title
                .font_family
                .clone()
                .unwrap_or_else(|| theme.title_font_family().to_string())
                .into(),
            font_size: title.font_size.unwrap_or(theme.title_font_size()).into(),
            font_weight: avenger_text::types::FontWeight::Number(theme.title_font_weight()).into(),
            color: crate::utils::parse_color_string(&theme.title_color())
                .unwrap_or(ColorOrGradient::Color([0.102, 0.102, 0.102, 1.0]))
                .into(),
            ..Default::default()
        };

        let group = SceneGroup {
            marks: vec![SceneMark::Text(text.into())],
            zindex: Some(20),
            ..Default::default()
        };
        Ok(vec![SceneMark::Group(group)])
    }

    /// Create subtitle mark if configured
    pub(super) fn create_subtitle(
        &self,
        _total_width: f32,
        _padding: &Padding,
        layout_bounds: Option<crate::layout::LayoutBounds>,
        _plot_area: Option<crate::layout::LayoutBounds>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(subtitle) = self.plot.get_subtitle() else {
            return Ok(Vec::new());
        };

        // Get theme for typography
        let theme = self.plot.get_theme();

        // If we have layout bounds from Taffy, place the subtitle left-aligned within its bounds
        let (x, y) = if let Some(bounds) = layout_bounds {
            // Use the subtitle node's x position, not the plot area's
            (bounds.x, bounds.y + bounds.height / 2.0)
        } else {
            // Fallback: left-aligned below title
            (10.0, 30.0)
        };

        let text = SceneTextMark {
            text: subtitle.text.clone().into(),
            x: x.into(),
            y: y.into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            font: subtitle
                .font_family
                .clone()
                .unwrap_or_else(|| theme.subtitle_font_family().to_string())
                .into(),
            font_size: subtitle
                .font_size
                .unwrap_or(theme.subtitle_font_size())
                .into(),
            font_weight: avenger_text::types::FontWeight::Number(theme.subtitle_font_weight())
                .into(),
            color: crate::utils::parse_color_string(&theme.subtitle_color())
                .unwrap_or(ColorOrGradient::Color([0.290, 0.290, 0.290, 1.0]))
                .into(),
            ..Default::default()
        };

        let group = SceneGroup {
            marks: vec![SceneMark::Text(text.into())],
            zindex: Some(20),
            ..Default::default()
        };
        Ok(vec![SceneMark::Group(group)])
    }
}
