//! Title and subtitle rendering for CompiledPlot

use std::sync::Arc;

use avenger_scenegraph::marks::mark::SceneMark;

use crate::error::AvengerChartError;

use super::CompiledPlot;

impl CompiledPlot {
    /// Create title mark if configured
    pub(super) fn create_title(
        &self,
        layout_bounds: Option<crate::layout::LayoutBounds>,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(title) = &self.title else {
            return Ok(Vec::new());
        };

        let theme = self.get_theme();
        let title_ctx = crate::theme::ThemeContext::new("chart-title").with_params(params.clone());

        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{FontStyle, TextAlign, TextBaseline};

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
            color: avenger_common::types::ColorOrGradient::Color(
                theme.text_color(&title_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]),
            )
            .into(),
            font_size: title
                .font_size
                .or_else(|| theme.font_size(&title_ctx))
                .unwrap_or(16.0)
                .into(),
            font: title
                .font_family
                .clone()
                .or_else(|| theme.font_family(&title_ctx))
                .unwrap_or_else(|| "sans-serif".to_string())
                .into(),
            font_style: FontStyle::Normal.into(),
            font_weight: avenger_text::types::FontWeight::Number(
                theme.font_weight(&title_ctx).unwrap_or(400.0),
            )
            .into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            ..Default::default()
        };

        Ok(vec![SceneMark::Text(Arc::new(text_mark))])
    }

    /// Create subtitle mark if configured
    pub(super) fn create_subtitle(
        &self,
        layout_bounds: Option<crate::layout::LayoutBounds>,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(subtitle) = &self.subtitle else {
            return Ok(Vec::new());
        };

        let theme = self.get_theme();
        let subtitle_ctx =
            crate::theme::ThemeContext::new("chart-subtitle").with_params(params.clone());

        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{FontStyle, TextAlign, TextBaseline};

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
            color: avenger_common::types::ColorOrGradient::Color(
                theme
                    .text_color(&subtitle_ctx)
                    .unwrap_or([0.0, 0.0, 0.0, 1.0]),
            )
            .into(),
            font_size: subtitle
                .font_size
                .or_else(|| theme.font_size(&subtitle_ctx))
                .unwrap_or(14.0)
                .into(),
            font: subtitle
                .font_family
                .clone()
                .or_else(|| theme.font_family(&subtitle_ctx))
                .unwrap_or_else(|| "sans-serif".to_string())
                .into(),
            font_style: FontStyle::Normal.into(),
            font_weight: avenger_text::types::FontWeight::Number(
                theme.font_weight(&subtitle_ctx).unwrap_or(400.0),
            )
            .into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Middle.into(),
            ..Default::default()
        };

        Ok(vec![SceneMark::Text(Arc::new(text_mark))])
    }
}
