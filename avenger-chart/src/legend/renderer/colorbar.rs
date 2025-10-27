//! Colorbar legend renderer for continuous color scales

use super::{LegendChannel, LegendRenderer};
use crate::error::AvengerChartError;
use crate::legend::Legend;
use crate::plot::compiled::expr_eval::{
    evaluate_f32_expr, evaluate_f64_expr, evaluate_legend_position_expr, evaluate_string_expr,
};
use crate::scales::{ConfiguredScaleLegendExt, DomainValues};
use avenger_guides::legend::colorbar::{ColorbarConfig, ColorbarOrientation};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_geometry::rtree::EnvelopeUtils;
use avenger_scenegraph::marks::group::SceneGroup;
use serde::{Deserialize, Serialize};

/// Colorbar legend renderer for continuous color scales
#[derive(Default, Serialize, Deserialize)]
pub struct CompiledColorbar;

impl CompiledColorbar {
    /// Default gradient thickness in pixels
    const DEFAULT_GRADIENT_THICKNESS: f64 = 15.0;

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

    fn can_evaluate(&self, channels: &[LegendChannel]) -> bool {
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

    async fn evaluate(
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

        // Evaluate gradient_thickness expression (from expression, theme, or default)
        use crate::serialization::LogicalExprNodeExt;

        let gradient_thickness = if let Some(node) = config
            .gradient_thickness
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            evaluate_f64_expr(&expr, ctx, params).await?
        } else {
            // Query from theme legend context
            let legend_ctx = theme.legend_context_with_params(Some("colorbar"), params.clone());
            theme
                .query(&legend_ctx, "gradient-thickness")
                .and_then(|v| v.as_font_size(params, theme.get_base_font_size(params)))
                .map(|f| f as f64)
                .unwrap_or(Self::DEFAULT_GRADIENT_THICKNESS)
        };

        // Evaluate position to determine orientation
        let position = if let Some(node) = config.position.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            evaluate_legend_position_expr(&expr, ctx, params).await?
        } else {
            // Position not set - check theme with runtime params for media queries
            let legend_ctx = theme.legend_context_with_params(Some("colorbar"), params.clone());
            if let Some(theme_value) = theme.query(&legend_ctx, "position") {
                if let Some(position_str) = theme_value.as_string() {
                    match position_str.to_lowercase().as_str() {
                        "top" => crate::legend::LegendPosition::Top,
                        "bottom" => crate::legend::LegendPosition::Bottom,
                        "left" => crate::legend::LegendPosition::Left,
                        "right" => crate::legend::LegendPosition::Right,
                        _ => crate::legend::LegendPosition::Right,
                    }
                } else {
                    crate::legend::LegendPosition::Right
                }
            } else {
                crate::legend::LegendPosition::Right // default
            }
        };

        // Map position to ColorbarOrientation
        let orientation = match position {
            crate::legend::LegendPosition::Top => ColorbarOrientation::Top,
            crate::legend::LegendPosition::Bottom => ColorbarOrientation::Bottom,
            crate::legend::LegendPosition::Left => ColorbarOrientation::Left,
            crate::legend::LegendPosition::Right => ColorbarOrientation::Right,
        };

        // Determine colorbar dimensions based on orientation
        // Always use available space for gradient length (flexible layout)
        // Note: In ColorbarConfig, both width and height represent different things depending on orientation:
        // - For all orientations: colorbar_height is the LENGTH, colorbar_width is the THICKNESS
        let (colorbar_height_param, colorbar_width_param) = match orientation {
            ColorbarOrientation::Left | ColorbarOrientation::Right => {
                // Vertical: height is length, width is thickness
                (Some(height), Some(gradient_thickness as f32))
            }
            ColorbarOrientation::Top | ColorbarOrientation::Bottom => {
                // Horizontal: height is still length (not thickness!), width is still thickness
                (Some(width), Some(gradient_thickness as f32))
            }
        };

        let mut legend_config = ColorbarConfig {
            orientation,
            dimensions: [width, height], // Available space for the colorbar
            colorbar_width: colorbar_width_param,
            colorbar_height: colorbar_height_param,
            colorbar_margin: Some(0.0), // No margin - align exactly with axis
            format_number: None,        // Will be set below after evaluating expression
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

        // Create legend context for theme queries
        let legend_ctx = theme.legend_context_with_params(Some("colorbar"), params.clone());

        // Evaluate format_number expression
        if let Some(node) = config.format_number.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            legend_config.format_number = Some(evaluate_string_expr(&expr, ctx, params).await?);
        }

        // Debug: print colorbar configuration if requested
        if std::env::var("AVENGER_DEBUG_COLORBAR").is_ok() {
            eprintln!(
                "COLORBAR DEBUG: orient={:?} thickness={:?}",
                legend_config.orientation, legend_config.colorbar_width
            );
        }

        // Evaluate and apply legend colors (from config or theme)
        // Title color
        if let Some(node) = config.title_color.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let title_color_str = evaluate_string_expr(&expr, ctx, params).await?;
            if let Ok(avenger_common::types::ColorOrGradient::Color(c)) =
                crate::utils::parse_color_string_strict(&title_color_str)
            {
                legend_config.title_color = Some(c);
            }
        } else {
            // Check theme for title color
            let title_ctx = legend_ctx.child("title");
            if let Some(color_value) = theme.query(&title_ctx, "color") {
                if let Some(color_str) = color_value.as_string() {
                    if let Ok(avenger_common::types::ColorOrGradient::Color(c)) =
                        crate::utils::parse_color_string_strict(&color_str)
                    {
                        legend_config.title_color = Some(c);
                    }
                }
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
        } else {
            // Check theme for tick color
            let tick_ctx = legend_ctx.child("tick");
            if let Some(color_value) = theme.query(&tick_ctx, "color") {
                if let Some(color_str) = color_value.as_string() {
                    if let Ok(avenger_common::types::ColorOrGradient::Color(c)) =
                        crate::utils::parse_color_string_strict(&color_str)
                    {
                        legend_config.label_color = Some(c);
                    }
                }
            }
        }

        // Evaluate and set typography (from config or theme)
        // Title font family
        if let Some(node) = config
            .title_font_family
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            legend_config.title_font_family = Some(evaluate_string_expr(&expr, ctx, params).await?);
        } else {
            // Check theme for title font family
            let title_ctx = legend_ctx.child("title");
            if let Some(family_value) = theme.query(&title_ctx, "font-family") {
                if let Some(family_str) = family_value.as_string() {
                    legend_config.title_font_family = Some(family_str.to_string());
                }
            }
        }

        // Title font size
        if let Some(node) = config.title_font_size.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            legend_config.title_font_size = Some(evaluate_f32_expr(&expr, ctx, params).await?);
        } else {
            // Check theme for title font size if not in config
            let title_ctx = legend_ctx.child("title");
            if let Some(size) = theme.font_size(&title_ctx) {
                legend_config.title_font_size = Some(size);
            }
        }

        // Also check for label font size from theme
        let tick_ctx = legend_ctx.child("tick");
        if let Some(size) = theme.font_size(&tick_ctx) {
            legend_config.label_font_size = Some(size);
        }

        // Title font weight
        if let Some(node) = config
            .title_font_weight
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            let weight = evaluate_f32_expr(&expr, ctx, params).await?;
            legend_config.title_font_weight = Some(avenger_text::types::FontWeight::Number(weight));
        } else {
            // Check theme for title font weight
            let title_ctx = legend_ctx.child("title");
            if let Some(weight) = theme.font_weight(&title_ctx) {
                legend_config.title_font_weight =
                    Some(avenger_text::types::FontWeight::Number(weight));
            }
        }

        // Use tick typography for colorbar axis labels
        // Tick font family
        if let Some(node) = config.tick_font_family.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            legend_config.label_font_family = Some(evaluate_string_expr(&expr, ctx, params).await?);
        } else {
            // Check theme for tick font family
            let tick_ctx = legend_ctx.child("tick");
            if let Some(family_value) = theme.query(&tick_ctx, "font-family") {
                if let Some(family_str) = family_value.as_string() {
                    legend_config.label_font_family = Some(family_str.to_string());
                }
            }
        }

        // Tick font weight
        if let Some(node) = config.tick_font_weight.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let weight = evaluate_f32_expr(&expr, ctx, params).await?;
            legend_config.label_font_weight = Some(avenger_text::types::FontWeight::Number(weight));
        } else {
            // Check theme for tick font weight
            let tick_ctx = legend_ctx.child("tick");
            if let Some(weight) = theme.font_weight(&tick_ctx) {
                legend_config.label_font_weight =
                    Some(avenger_text::types::FontWeight::Number(weight));
            }
        }

        // Evaluate and apply legend background styling (from config or theme)
        // Background padding
        if let Some(node) = config
            .background_padding
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            legend_config.background_padding = Some(evaluate_f32_expr(&expr, ctx, params).await?);
        } else {
            // Check theme for background padding
            let background_ctx = legend_ctx.child("background");
            if let Some(padding) = theme
                .query(&background_ctx, "padding")
                .and_then(|v| v.as_font_size(params, theme.get_base_font_size(params)))
            {
                legend_config.background_padding = Some(padding);
            }
        }

        // Background corner radius
        if let Some(node) = config
            .background_corner_radius
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            legend_config.background_corner_radius =
                Some(evaluate_f32_expr(&expr, ctx, params).await?);
        } else {
            // Check theme for corner radius
            let background_ctx = legend_ctx.child("background");
            if let Some(radius) = theme
                .query(&background_ctx, "corner-radius")
                .and_then(|v| v.as_font_size(params, theme.get_base_font_size(params)))
            {
                legend_config.background_corner_radius = Some(radius);
            }
        }

        // Background fill
        if let Some(node) = config.background_fill.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let fill_str = evaluate_string_expr(&expr, ctx, params).await?;
            if let Some(color) = crate::utils::parse_color_string(&fill_str) {
                legend_config.background_fill = Some(color);
            }
        } else {
            // Check theme for background fill
            let background_ctx = legend_ctx.child("background");
            if let Some(fill_value) = theme.query(&background_ctx, "fill") {
                if let Some(fill_str) = fill_value.as_string() {
                    if let Some(color) = crate::utils::parse_color_string(&fill_str) {
                        legend_config.background_fill = Some(color);
                    }
                }
            }
        }

        // Background stroke
        if let Some(node) = config
            .background_stroke
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            let stroke_str = evaluate_string_expr(&expr, ctx, params).await?;
            if let Some(color) = crate::utils::parse_color_string(&stroke_str) {
                legend_config.background_stroke = Some(color);
            }
        } else {
            // Check theme for background stroke
            let background_ctx = legend_ctx.child("background");
            if let Some(stroke_value) = theme.query(&background_ctx, "stroke") {
                if let Some(stroke_str) = stroke_value.as_string() {
                    if let Some(color) = crate::utils::parse_color_string(&stroke_str) {
                        legend_config.background_stroke = Some(color);
                    }
                }
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

        if std::env::var("AVENGER_DEBUG_COLORBAR_GROUP").is_ok() {
            let bbox = colorbar_group.bounding_box();
            let lower = bbox.lower();
            let upper = bbox.upper();
            eprintln!(
                "COLORBAR GROUP BBOX: lower=({:.1},{:.1}) upper=({:.1},{:.1}) w={:.1} h={:.1}",
                lower[0], lower[1], upper[0], upper[1], bbox.width(), bbox.height()
            );
        }

        // Position the colorbar group
        colorbar_group.origin = [x, y];

        // Set z-index
        colorbar_group.zindex = Some(10); // Legends above data but below title

        Ok(Some(colorbar_group))
    }
}
