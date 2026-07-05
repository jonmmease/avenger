//! Colorbar legend renderer for continuous color scales

use avenger_chart_core::{
    AvengerChartError, LayoutBounds, Legend, LegendContinuousOrientation, LegendContinuousSurface,
    LegendPosition, LegendRenderOutput, LegendSurfaceKind, Theme, evaluate_f32_expr,
    evaluate_f64_expr, evaluate_legend_position_expr, evaluate_string_expr,
};
use avenger_chart_core::{ConfiguredScaleLegendExt, DefaultLogicalExprNodeExt, DomainValues};
use avenger_color::parse_color_string_strict;
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_guides::legend::{
    GuideLegendContinuousOrientation,
    colorbar::{ColorbarConfig, ColorbarOrientation, make_colorbar_marks_with_surfaces},
};
use avenger_scales::scales::{ConfiguredScale, DomainKind, RangeKind, linear::LinearScale};
use avenger_text::types::FontWeight;
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use super::{LegendChannel, LegendRenderer, parse_color_or_gradient};

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

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[typetag::serde]
impl LegendRenderer for CompiledColorbar {
    fn name(&self) -> &'static str {
        "CompiledColorbar"
    }

    fn prefers_flexible_layout(&self) -> bool {
        true // Colorbars can stretch vertically
    }

    fn can_evaluate(&self, channels: &[LegendChannel]) -> bool {
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
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Option<LegendRenderOutput>, AvengerChartError> {
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
                        "top" => LegendPosition::Top,
                        "bottom" => LegendPosition::Bottom,
                        "left" => LegendPosition::Left,
                        "right" => LegendPosition::Right,
                        _ => LegendPosition::Right,
                    }
                } else {
                    LegendPosition::Right
                }
            } else {
                LegendPosition::Right // default
            }
        };

        // Map position to ColorbarOrientation
        let orientation = match position {
            LegendPosition::Top => ColorbarOrientation::Top,
            LegendPosition::Bottom => ColorbarOrientation::Bottom,
            LegendPosition::Left => ColorbarOrientation::Left,
            LegendPosition::Right => ColorbarOrientation::Right,
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
            number_locale: None,
            number_locale_registry: None,
            number_locale_specs: avenger_text::NumberLocaleSpecs::default(),
            background_fill: None,
            background_stroke: None,
            background_corner_radius: None,
            background_padding: None,
            title_font_family: None,
            title_font_size: None,
            title_font_weight: None,
            title_color: None,
            title_syntax_mode: config.title_syntax_mode,
            title_text_params: avenger_text::LabelParams::default(),
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

        debug!(
            orientation = ?legend_config.orientation,
            thickness = ?legend_config.colorbar_width,
            "Colorbar config"
        );

        // Evaluate and apply legend colors (from config or theme)
        // Title color
        if let Some(node) = config.title_color.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let title_color_str = evaluate_string_expr(&expr, ctx, params).await?;
            if let Ok(c) = parse_color_string_strict(&title_color_str) {
                legend_config.title_color = Some(c);
            }
        } else {
            // Check theme for title color
            let title_ctx = legend_ctx.child("title");
            if let Some(color_value) = theme.query(&title_ctx, "color")
                && let Some(color_str) = color_value.as_string()
                && let Ok(c) = parse_color_string_strict(color_str)
            {
                legend_config.title_color = Some(c);
            }
        }

        // Use tick_color for colorbar axis labels (not label_color which is for discrete legends)
        if let Some(node) = config.tick_color.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let tick_color_str = evaluate_string_expr(&expr, ctx, params).await?;
            if let Ok(c) = parse_color_string_strict(&tick_color_str) {
                legend_config.label_color = Some(c);
            }
        } else {
            // Check theme for tick color
            let tick_ctx = legend_ctx.child("tick");
            if let Some(color_value) = theme.query(&tick_ctx, "color")
                && let Some(color_str) = color_value.as_string()
                && let Ok(c) = parse_color_string_strict(color_str)
            {
                legend_config.label_color = Some(c);
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
            if let Some(family_value) = theme.query(&title_ctx, "font-family")
                && let Some(family_str) = family_value.as_string()
            {
                legend_config.title_font_family = Some(family_str.to_string());
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
            legend_config.title_font_weight = Some(FontWeight::Number(weight));
        } else {
            // Check theme for title font weight
            let title_ctx = legend_ctx.child("title");
            if let Some(weight) = theme.font_weight(&title_ctx) {
                legend_config.title_font_weight = Some(FontWeight::Number(weight));
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
            if let Some(family_value) = theme.query(&tick_ctx, "font-family")
                && let Some(family_str) = family_value.as_string()
            {
                legend_config.label_font_family = Some(family_str.to_string());
            }
        }

        // Tick font weight
        if let Some(node) = config.tick_font_weight.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let weight = evaluate_f32_expr(&expr, ctx, params).await?;
            legend_config.label_font_weight = Some(FontWeight::Number(weight));
        } else {
            // Check theme for tick font weight
            let tick_ctx = legend_ctx.child("tick");
            if let Some(weight) = theme.font_weight(&tick_ctx) {
                legend_config.label_font_weight = Some(FontWeight::Number(weight));
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
            if let Some(color) = parse_color_or_gradient(&fill_str) {
                legend_config.background_fill = Some(color);
            }
        } else {
            // Check theme for background fill
            let background_ctx = legend_ctx.child("background");
            if let Some(fill_value) = theme.query(&background_ctx, "fill")
                && let Some(fill_str) = fill_value.as_string()
                && let Some(color) = parse_color_or_gradient(fill_str)
            {
                legend_config.background_fill = Some(color);
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
            if let Some(color) = parse_color_or_gradient(&stroke_str) {
                legend_config.background_stroke = Some(color);
            }
        } else {
            // Check theme for background stroke
            let background_ctx = legend_ctx.child("background");
            if let Some(stroke_value) = theme.query(&background_ctx, "stroke")
                && let Some(stroke_str) = stroke_value.as_string()
                && let Some(color) = parse_color_or_gradient(stroke_str)
            {
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
        legend_config.title_text_params = avenger_chart_core::scalar_params_for_label_source(
            &title,
            config.title_syntax_mode,
            params,
        )?;

        // Create the colorbar marks at origin [0, 0] (will be positioned by group origin)
        let plot_origin = [0.0, 0.0];

        let mut colorbar_output = make_colorbar_marks_with_surfaces(
            configured_scale,
            &title,
            plot_origin,
            &legend_config,
        )?;

        let bbox = colorbar_output.group.bounding_box();
        let lower = bbox.lower();
        let upper = bbox.upper();
        trace!(
            lower_x = lower[0],
            lower_y = lower[1],
            upper_x = upper[0],
            upper_y = upper[1],
            width = bbox.width(),
            height = bbox.height(),
            "Colorbar group bbox"
        );

        // Position the colorbar group
        colorbar_output.group.origin = [x, y];

        // Set z-index
        colorbar_output.group.zindex = Some(10); // Legends above data but below title

        let continuous_surfaces = colorbar_output
            .continuous_surfaces
            .into_iter()
            .map(|surface| {
                let bounds = LayoutBounds {
                    x: surface.bounds[0],
                    y: surface.bounds[1],
                    width: surface.bounds[2],
                    height: surface.bounds[3],
                };
                let (value_scale, band_scale) = colorbar_surface_scales(
                    configured_scale,
                    surface.value_channel.as_str(),
                    bounds,
                );
                LegendContinuousSurface {
                    channel: primary_channel.channel_type.clone(),
                    name: primary_channel.name.clone(),
                    legend_id: config.id.clone(),
                    surface_key: primary_channel.name.clone(),
                    kind: LegendSurfaceKind::ContinuousColorbar,
                    orientation: match surface.orientation {
                        GuideLegendContinuousOrientation::Top => LegendContinuousOrientation::Top,
                        GuideLegendContinuousOrientation::Bottom => {
                            LegendContinuousOrientation::Bottom
                        }
                        GuideLegendContinuousOrientation::Left => LegendContinuousOrientation::Left,
                        GuideLegendContinuousOrientation::Right => {
                            LegendContinuousOrientation::Right
                        }
                    },
                    surface_group_path: surface.surface_group_path,
                    gradient_rect_path: surface.gradient_rect_path,
                    hit_rect_path: surface.hit_rect_path,
                    bounds,
                    value_channel: surface.value_channel,
                    band_channel: surface.band_channel,
                    value_scale,
                    band_scale,
                }
            })
            .collect();

        Ok(Some(LegendRenderOutput {
            group: colorbar_output.group,
            items: Vec::new(),
            continuous_surfaces,
        }))
    }
}

fn colorbar_surface_scales(
    configured_scale: &ConfiguredScale,
    value_channel: &str,
    bounds: LayoutBounds,
) -> (ConfiguredScale, ConfiguredScale) {
    let value_range = if value_channel == "x" {
        (0.0, bounds.width)
    } else {
        (bounds.height, 0.0)
    };
    let cross_range = if value_channel == "x" {
        (0.0, bounds.height)
    } else {
        (0.0, bounds.width)
    };
    let value_scale = configured_scale.clone().with_range_interval(value_range);
    let cross_scale = LinearScale::configured((0.0, 1.0), cross_range);
    (value_scale, cross_scale)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use avenger_chart_core::{
        LegendChannel, LegendContinuousOrientation, LegendPosition, LegendSurfaceKind,
    };
    use avenger_scales::scales::linear::LinearScale;

    use super::*;

    #[test]
    fn colorbar_renderer_emits_continuous_surface_metadata() {
        futures::executor::block_on(async {
            let renderer = CompiledColorbar::new();
            let scale = LinearScale::configured_color((0.0, 100.0), ["#440154", "#fde725"]);
            let channel = LegendChannel {
                name: "fill".to_string(),
                expression: None,
                scale,
                channel_type: "fill".to_string(),
                sharing_level: None,
                mark_type: "symbol".to_string(),
                mark_index: 0,
                pattern_range: None,
                related_channels: HashMap::new(),
            };
            let config = Legend::new()
                .id("temperature")
                .position(LegendPosition::Right)
                .title("Temperature");
            let theme = Theme::light();
            let params = IndexMap::new();
            let ctx = SessionContext::new();
            let output = renderer
                .evaluate(
                    &[channel],
                    &config,
                    0.0,
                    0.0,
                    80.0,
                    220.0,
                    &theme,
                    &params,
                    &ctx,
                )
                .await
                .expect("colorbar evaluates")
                .expect("colorbar output");

            assert!(output.items.is_empty());
            assert_eq!(output.continuous_surfaces.len(), 1);
            let surface = &output.continuous_surfaces[0];
            assert_eq!(surface.kind, LegendSurfaceKind::ContinuousColorbar);
            assert_eq!(surface.orientation, LegendContinuousOrientation::Right);
            assert_eq!(surface.channel, "fill");
            assert_eq!(surface.name, "fill");
            assert_eq!(surface.legend_id.as_deref(), Some("temperature"));
            assert_eq!(surface.surface_key, "fill");
            assert_eq!(surface.value_channel, "y");
            assert_eq!(surface.band_channel, "x");
            assert_eq!(surface.hit_rect_path, surface.gradient_rect_path);
            assert!(surface.bounds.width > 0.0);
            assert!(surface.bounds.height > 0.0);

            let midpoint = surface
                .value_scale
                .invert_scalar(surface.bounds.height * 0.5)
                .expect("value scale inverts");
            assert!((midpoint - 50.0).abs() < 1.0);
        });
    }
}
