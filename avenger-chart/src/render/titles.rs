//! Title and subtitle rendering
//!
//! This module handles:
//! - Creating title marks with proper positioning
//! - Creating subtitle marks with proper positioning
//! - Applying theme settings to text elements

use super::PlotRenderer;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::layout::LayoutBounds;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_text::types::{FontStyle, TextAlign, TextBaseline};
use std::sync::Arc;

impl<C: CoordinateSystem> PlotRenderer<'_, C> {
    /// Create title mark if configured
    pub(super) fn create_title(
        &self,
        layout_bounds: Option<LayoutBounds>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(title) = self.plot.get_title() else {
            return Ok(Vec::new());
        };

        let theme = self.plot.get_theme();

        // Position title within its layout bounds or use fallback
        let (x, y) = if let Some(bounds) = layout_bounds {
            (bounds.x, bounds.y + bounds.height / 2.0)
        } else {
            (10.0, 20.0)
        };

        let text_mark = SceneTextMark {
            text: title.text.clone().into(),
            x: x.into(),
            y: y.into(),
            color: crate::utils::parse_color_string(&theme.title_color())
                .unwrap_or(avenger_common::types::ColorOrGradient::Color([
                    0.102, 0.102, 0.102, 1.0,
                ]))
                .into(),
            font_size: title.font_size.unwrap_or(theme.title_font_size()).into(),
            font: title
                .font_family
                .clone()
                .unwrap_or_else(|| theme.title_font_family())
                .into(),
            font_style: FontStyle::Normal.into(),
            font_weight: avenger_text::types::FontWeight::Number(theme.title_font_weight()).into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            ..Default::default()
        };

        Ok(vec![SceneMark::Text(Arc::new(text_mark))])
    }

    /// Create subtitle mark if configured
    pub(super) fn create_subtitle(
        &self,
        layout_bounds: Option<LayoutBounds>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(subtitle) = self.plot.get_subtitle() else {
            return Ok(Vec::new());
        };

        let theme = self.plot.get_theme();

        // Position subtitle within its layout bounds or use fallback
        let (x, y) = if let Some(bounds) = layout_bounds {
            (bounds.x, bounds.y + bounds.height / 2.0)
        } else {
            (10.0, 40.0)
        };

        let text_mark = SceneTextMark {
            text: subtitle.text.clone().into(),
            x: x.into(),
            y: y.into(),
            color: crate::utils::parse_color_string(&theme.subtitle_color())
                .unwrap_or(avenger_common::types::ColorOrGradient::Color([
                    0.290, 0.290, 0.290, 1.0,
                ]))
                .into(),
            font_size: subtitle
                .font_size
                .unwrap_or(theme.subtitle_font_size())
                .into(),
            font: subtitle
                .font_family
                .clone()
                .unwrap_or_else(|| theme.subtitle_font_family())
                .into(),
            font_style: FontStyle::Normal.into(),
            font_weight: avenger_text::types::FontWeight::Number(theme.subtitle_font_weight())
                .into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            ..Default::default()
        };

        Ok(vec![SceneMark::Text(Arc::new(text_mark))])
    }
}
