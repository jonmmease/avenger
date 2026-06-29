//! Title and subtitle rendering for CompiledPlot

use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_scenegraph::marks::mark::SceneMark;

use avenger_chart_core::{
    AvengerChartError, LayoutBounds, ThemeContext, evaluate_f32_expr, evaluate_string_expr,
};

use super::CompiledPlot;

impl CompiledPlot {
    // Default values for title/subtitle rendering
    const DEFAULT_TITLE_FONT_SIZE: f32 = 16.0;
    const DEFAULT_SUBTITLE_FONT_SIZE: f32 = 14.0;
    const DEFAULT_FONT_FAMILY: &'static str = "sans-serif";
    const DEFAULT_FONT_WEIGHT: f32 = 400.0;
    const DEFAULT_TEXT_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0]; // Black
    const FALLBACK_TITLE_POSITION: (f32, f32) = (10.0, 20.0);
    const FALLBACK_SUBTITLE_POSITION: (f32, f32) = (10.0, 40.0);

    /// Create title mark if configured
    pub(super) async fn create_title(
        &self,
        layout_bounds: Option<LayoutBounds>,
        ctx: &datafusion::prelude::SessionContext,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(title) = &self.title else {
            return Ok(Vec::new());
        };

        let theme = self.get_theme();
        let title_ctx = ThemeContext::new("chart-title", params.clone());

        use avenger_chart_scales::serialization::LogicalExprNodeExt;
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{FontStyle, TextAlign, TextBaseline};
        use datafusion_proto::protobuf::LogicalExprNode;

        // Evaluate the title text expression
        let text_node: LogicalExprNode = title.text.clone();
        let text_expr = text_node.to_expr(ctx)?;
        let text_value = evaluate_string_expr(&text_expr, ctx, params).await?;
        let text_params = avenger_chart_core::scalar_params_for_label_source(
            &text_value,
            title.syntax_mode,
            params,
        )?;

        // Evaluate font_size
        let font_size = match title.font_size.as_ref() {
            avenger_chart_core::maybe::Maybe::Set(Some(node)) => {
                let expr = node.to_expr(ctx)?;
                evaluate_f32_expr(&expr, ctx, params).await?
            }
            _ => theme
                .font_size(&title_ctx)
                .unwrap_or(Self::DEFAULT_TITLE_FONT_SIZE),
        };

        // Evaluate font_family
        let font_family = match title.font_family.as_ref() {
            avenger_chart_core::maybe::Maybe::Set(Some(node)) => {
                let expr = node.to_expr(ctx)?;
                evaluate_string_expr(&expr, ctx, params).await?
            }
            _ => theme
                .font_family(&title_ctx)
                .unwrap_or_else(|| Self::DEFAULT_FONT_FAMILY.to_string()),
        };

        // Evaluate text alignment (from expression or theme)
        let text_align = match title.align.as_ref() {
            avenger_chart_core::maybe::Maybe::Set(Some(node)) => {
                let expr = node.to_expr(ctx)?;
                let align_str = evaluate_string_expr(&expr, ctx, params).await?;
                match align_str.to_lowercase().as_str() {
                    "left" => TextAlign::Left,
                    "center" => TextAlign::Center,
                    "right" => TextAlign::Right,
                    _ => TextAlign::default(),
                }
            }
            _ => {
                // Query from theme
                theme
                    .text_align(&title_ctx)
                    .and_then(|s| match s.to_lowercase().as_str() {
                        "left" => Some(TextAlign::Left),
                        "center" => Some(TextAlign::Center),
                        "right" => Some(TextAlign::Right),
                        _ => None,
                    })
                    .unwrap_or_default()
            }
        };

        // Position title within its layout bounds or use fallback
        let (x, y) = if let Some(bounds) = layout_bounds {
            // Adjust x position based on text alignment
            let x_pos = match text_align {
                TextAlign::Left => bounds.x,
                TextAlign::Center => bounds.x + bounds.width / 2.0,
                TextAlign::Right => bounds.x + bounds.width,
            };
            (x_pos, bounds.y + bounds.height / 2.0)
        } else {
            Self::FALLBACK_TITLE_POSITION
        };

        let text_mark = SceneTextMark {
            clip: false,
            text: text_value.into(),
            text_syntax: title.syntax_mode,
            text_params,
            x: x.into(),
            y: y.into(),
            color: ColorOrGradient::Color(
                theme
                    .text_color(&title_ctx)
                    .unwrap_or(Self::DEFAULT_TEXT_COLOR),
            )
            .into(),
            font_size: font_size.into(),
            font: font_family.into(),
            font_style: FontStyle::Normal.into(),
            font_weight: avenger_text::types::FontWeight::Number(
                theme
                    .font_weight(&title_ctx)
                    .unwrap_or(Self::DEFAULT_FONT_WEIGHT),
            )
            .into(),
            align: text_align.into(),
            baseline: TextBaseline::Middle.into(),
            ..Default::default()
        };

        Ok(vec![SceneMark::Text(Arc::new(text_mark))])
    }

    /// Create subtitle mark if configured
    pub(super) async fn create_subtitle(
        &self,
        layout_bounds: Option<LayoutBounds>,
        ctx: &datafusion::prelude::SessionContext,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(subtitle) = &self.subtitle else {
            return Ok(Vec::new());
        };

        let theme = self.get_theme();
        let subtitle_ctx = ThemeContext::new("chart-subtitle", params.clone());

        use avenger_chart_scales::serialization::LogicalExprNodeExt;
        use avenger_scenegraph::marks::text::SceneTextMark;
        use avenger_text::types::{FontStyle, TextAlign, TextBaseline};
        use datafusion_proto::protobuf::LogicalExprNode;

        // Evaluate the subtitle text expression
        let text_node: LogicalExprNode = subtitle.text.clone();
        let text_expr = text_node.to_expr(ctx)?;
        let text_value = evaluate_string_expr(&text_expr, ctx, params).await?;
        let text_params = avenger_chart_core::scalar_params_for_label_source(
            &text_value,
            subtitle.syntax_mode,
            params,
        )?;

        // Evaluate font_size
        let font_size = match subtitle.font_size.as_ref() {
            avenger_chart_core::maybe::Maybe::Set(Some(node)) => {
                let expr = node.to_expr(ctx)?;
                evaluate_f32_expr(&expr, ctx, params).await?
            }
            _ => theme
                .font_size(&subtitle_ctx)
                .unwrap_or(Self::DEFAULT_SUBTITLE_FONT_SIZE),
        };

        // Evaluate font_family
        let font_family = match subtitle.font_family.as_ref() {
            avenger_chart_core::maybe::Maybe::Set(Some(node)) => {
                let expr = node.to_expr(ctx)?;
                evaluate_string_expr(&expr, ctx, params).await?
            }
            _ => theme
                .font_family(&subtitle_ctx)
                .unwrap_or_else(|| Self::DEFAULT_FONT_FAMILY.to_string()),
        };

        // Evaluate text alignment (from expression or theme)
        let text_align = match subtitle.align.as_ref() {
            avenger_chart_core::maybe::Maybe::Set(Some(node)) => {
                let expr = node.to_expr(ctx)?;
                let align_str = evaluate_string_expr(&expr, ctx, params).await?;
                match align_str.to_lowercase().as_str() {
                    "left" => TextAlign::Left,
                    "center" => TextAlign::Center,
                    "right" => TextAlign::Right,
                    _ => TextAlign::default(),
                }
            }
            _ => {
                // Query from theme
                theme
                    .text_align(&subtitle_ctx)
                    .and_then(|s| match s.to_lowercase().as_str() {
                        "left" => Some(TextAlign::Left),
                        "center" => Some(TextAlign::Center),
                        "right" => Some(TextAlign::Right),
                        _ => None,
                    })
                    .unwrap_or_default()
            }
        };

        // Position subtitle within its layout bounds or use fallback
        let (x, y) = if let Some(bounds) = layout_bounds {
            // Adjust x position based on text alignment
            let x_pos = match text_align {
                TextAlign::Left => bounds.x,
                TextAlign::Center => bounds.x + bounds.width / 2.0,
                TextAlign::Right => bounds.x + bounds.width,
            };
            (x_pos, bounds.y + bounds.height / 2.0)
        } else {
            Self::FALLBACK_SUBTITLE_POSITION
        };

        let text_mark = SceneTextMark {
            clip: false,
            text: text_value.into(),
            text_syntax: subtitle.syntax_mode,
            text_params,
            x: x.into(),
            y: y.into(),
            color: ColorOrGradient::Color(
                theme
                    .text_color(&subtitle_ctx)
                    .unwrap_or(Self::DEFAULT_TEXT_COLOR),
            )
            .into(),
            font_size: font_size.into(),
            font: font_family.into(),
            font_style: FontStyle::Normal.into(),
            font_weight: avenger_text::types::FontWeight::Number(
                theme
                    .font_weight(&subtitle_ctx)
                    .unwrap_or(Self::DEFAULT_FONT_WEIGHT),
            )
            .into(),
            align: text_align.into(),
            baseline: TextBaseline::Middle.into(),
            ..Default::default()
        };

        Ok(vec![SceneMark::Text(Arc::new(text_mark))])
    }
}
