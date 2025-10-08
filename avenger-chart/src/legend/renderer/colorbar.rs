//! Colorbar legend renderer for continuous color scales

use super::{LegendChannel, LegendRenderer};
use crate::error::AvengerChartError;
use crate::legend::Legend;
use crate::plot::compiled::expr_eval::{evaluate_f32_expr, evaluate_f64_expr, evaluate_string_expr, evaluate_legend_position_expr};
use crate::scales::{ConfiguredScaleLegendExt, DomainValues};
use avenger_guides::legend::colorbar::{ColorbarConfig, ColorbarOrientation};
use avenger_scenegraph::marks::group::SceneGroup;
use serde::{Deserialize, Serialize};

/// Colorbar legend renderer for continuous color scales
#[derive(Default, Serialize, Deserialize)]
pub struct CompiledColorbar;

impl CompiledColorbar {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl LegendRenderer for CompiledColorbar {
    fn name(&self) -> &'static str {
        "CompiledColorbar"
    }

    fn prefers_flexible_layout(&self) -> bool {
        true // Colorbars can stretch vertically
    }

    fn can_render(&self, channels: &[LegendChannel]) -> bool {
        use avenger_scales::scales::{DomainKind, RangeKind};

        // Colorbar is for continuous color scales with numeric/temporal domains
        channels.iter().all(|c| {
            (c.channel_type == "fill" || c.channel_type == "stroke")
                && c.scale.scale_impl.range_kind() == RangeKind::Continuous
                && matches!(
                    c.scale.scale_impl.domain_kind(),
                    DomainKind::Numeric | DomainKind::Temporal
                )
        })
    }

    async fn render(
        &self,
        channels: &[LegendChannel],
        config: &Legend,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        theme: &crate::theme::Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<Option<SceneGroup>, AvengerChartError> {
        use avenger_guides::legend::colorbar::make_colorbar_marks;

        if channels.is_empty() {
            return Ok(None);
        }

        // Use the first color channel as primary
        let primary_channel = channels
            .iter()
            .find(|c| {
                c.channel_type == "fill" || c.channel_type == "stroke" || c.channel_type == "color"
            })
            .unwrap_or(&channels[0]);

        // Get the ConfiguredScale for this channel
        let configured_scale = &primary_channel.scale;

        // Verify we have a continuous domain
        match configured_scale.domain_values()? {
            DomainValues::Interval(_, _) => {}
            _ => {
                return Err(AvengerChartError::InternalError(
                    "Colorbar legend requires continuous domain".to_string(),
                ));
            }
        }

        // Evaluate gradient_thickness expression (default 15.0 if not set)
        use crate::serialization::LogicalExprNodeExt;

        let gradient_thickness = if let Some(node) = config.gradient_thickness.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            evaluate_f64_expr(&expr, ctx, params).await?
        } else {
            15.0
        };

        // Evaluate position to determine orientation
        let position = if let Some(node) = config.position.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            evaluate_legend_position_expr(&expr, ctx, params).await?
        } else {
            crate::legend::LegendPosition::Right // default
        };

        // Map position to ColorbarOrientation
        let orientation = match position {
            crate::legend::LegendPosition::Top => ColorbarOrientation::Top,
            crate::legend::LegendPosition::Bottom => ColorbarOrientation::Bottom,
            crate::legend::LegendPosition::Left => ColorbarOrientation::Left,
            crate::legend::LegendPosition::Right => ColorbarOrientation::Right,
        };

        // Determine colorbar dimensions based on orientation
        // For vertical orientations (Left/Right): use height for gradient length
        // For horizontal orientations (Top/Bottom): use width for gradient length
        let (colorbar_height_param, colorbar_width_param) = match orientation {
            ColorbarOrientation::Left | ColorbarOrientation::Right => {
                // Vertical: height is the gradient length, width is the thickness
                (Some(height), Some(gradient_thickness as f32))
            }
            ColorbarOrientation::Top | ColorbarOrientation::Bottom => {
                // Horizontal: width is the gradient length, height is the thickness
                (Some(width), Some(gradient_thickness as f32))
            }
        };

        let mut legend_config = ColorbarConfig {
            orientation,
            dimensions: [width, height], // Available space for the colorbar
            colorbar_width: colorbar_width_param,
            colorbar_height: colorbar_height_param,
            colorbar_margin: Some(0.0), // No margin - align exactly with axis
            format_number: None, // Will be set below after evaluating expression
            background_fill: None,
            background_stroke: None,
            background_corner_radius: None,
            background_padding: None,
            title_font_family: None,
            title_font_size: None,
            title_font_weight: None,
            title_color: None,
            label_font_family: None,
            label_font_size: None,
            label_font_weight: None,
            label_color: None,
            domain_color: None,
            tick_color: None,
        };

        // Evaluate format_number expression
        if let Some(node) = config.format_number.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            legend_config.format_number = Some(evaluate_string_expr(&expr, ctx, params).await?);
        }

        // Evaluate and apply legend colors from config
        if let Some(node) = config.title_color.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let title_color_str = evaluate_string_expr(&expr, ctx, params).await?;
            if let Ok(avenger_common::types::ColorOrGradient::Color(c)) =
                crate::utils::parse_color_string_strict(&title_color_str)
            {
                legend_config.title_color = Some(c);
            }
        }
        // Use tick_color for colorbar axis labels (not label_color which is for discrete legends)
        if let Some(node) = config.tick_color.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let tick_color_str = evaluate_string_expr(&expr, ctx, params).await?;
            if let Ok(avenger_common::types::ColorOrGradient::Color(c)) =
                crate::utils::parse_color_string_strict(&tick_color_str)
            {
                legend_config.label_color = Some(c);
            }
        }

        // Evaluate and set typography from legend config
        if let Some(node) = config.title_font_family.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            legend_config.title_font_family = Some(evaluate_string_expr(&expr, ctx, params).await?);
        }
        if let Some(node) = config.title_font_size.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            legend_config.title_font_size = Some(evaluate_f32_expr(&expr, ctx, params).await?);
        }
        // Override font sizes with params
        let legend_ctx = theme
            .legend_context(Some("colorbar"))
            .with_params(params.clone());
        let title_ctx = legend_ctx.child("title");
        let tick_ctx = legend_ctx.child("tick");

        if let Some(size) = theme.font_size(&title_ctx) {
            legend_config.title_font_size = Some(size);
        }
        if let Some(size) = theme.font_size(&tick_ctx) {
            legend_config.label_font_size = Some(size);
        }
        if let Some(node) = config.title_font_weight.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let weight = evaluate_f32_expr(&expr, ctx, params).await?;
            legend_config.title_font_weight = Some(avenger_text::types::FontWeight::Number(weight));
        }
        // Use tick typography for colorbar axis labels
        if let Some(node) = config.tick_font_family.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            legend_config.label_font_family = Some(evaluate_string_expr(&expr, ctx, params).await?);
        }
        if let Some(node) = config.tick_font_weight.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let weight = evaluate_f32_expr(&expr, ctx, params).await?;
            legend_config.label_font_weight = Some(avenger_text::types::FontWeight::Number(weight));
        }

        // Evaluate and apply legend background styling if provided
        if let Some(node) = config.background_padding.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            legend_config.background_padding = Some(evaluate_f32_expr(&expr, ctx, params).await?);
        }
        if let Some(node) = config.background_corner_radius.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            legend_config.background_corner_radius = Some(evaluate_f32_expr(&expr, ctx, params).await?);
        }
        if let Some(node) = config.background_fill.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let fill_str = evaluate_string_expr(&expr, ctx, params).await?;
            if let Some(color) = crate::utils::parse_color_string(&fill_str) {
                legend_config.background_fill = Some(color);
            }
        }
        if let Some(node) = config.background_stroke.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let stroke_str = evaluate_string_expr(&expr, ctx, params).await?;
            if let Some(color) = crate::utils::parse_color_string(&stroke_str) {
                legend_config.background_stroke = Some(color);
            }
        }

        // Evaluate title expression
        let title = if let Some(node) = config.title.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            evaluate_string_expr(&expr, ctx, params).await?
        } else {
            String::new()
        };

        // Create the colorbar marks at origin [0, 0] (will be positioned by group origin)
        let plot_origin = [0.0, 0.0];

        let mut colorbar_group =
            make_colorbar_marks(configured_scale, &title, plot_origin, &legend_config)?;

        // Position the colorbar group
        colorbar_group.origin = [x, y];

        // Set z-index
        colorbar_group.zindex = Some(10); // Legends above data but below title

        Ok(Some(colorbar_group))
    }
}
