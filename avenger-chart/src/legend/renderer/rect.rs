//! Rectangle/bar legend renderer for discrete scales

use std::collections::HashMap;

use avenger_common::{
    types::{ColorOrGradient, SymbolShape},
    value::ScalarOrArray,
};
use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend};
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_text::types::FontWeight;
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    channel::ChannelValue,
    error::AvengerChartError,
    legend::Legend,
    plot::compiled::expr_eval::{evaluate_f32_expr, evaluate_string_expr},
    scales::{ConfiguredScaleLegendExt, DomainValues},
    serialization::LogicalExprNodeExt,
    theme::Theme,
    utils::{ScalarValueHelpers, parse_color_string_strict},
};

use super::{LegendChannel, LegendRenderer, helpers};

/// Rectangle legend renderer for rect/bar marks
#[derive(Default, Serialize, Deserialize)]
pub struct CompiledRectLegend {
    /// Map of mark encodings from the plot
    mark_encodings: HashMap<String, ChannelValue>,
}

impl CompiledRectLegend {
    // Default values for rect legends
    const DEFAULT_SIZE: f32 = 64.0;
    const DEFAULT_ANGLE: f32 = 0.0;
    const DEFAULT_FILL: &'static str = "#4682b4"; // Steelblue
    const DEFAULT_STROKE: &'static str = "#000000"; // Black
    const DEFAULT_STROKE_WIDTH: f32 = 1.0;
    const INNER_HEIGHT: f32 = 100.0;
    const TEXT_PADDING: f32 = 2.0;

    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl LegendRenderer for CompiledRectLegend {
    fn name(&self) -> &'static str {
        "CompiledRectLegend"
    }

    fn can_evaluate(&self, channels: &[LegendChannel]) -> bool {
        // Can render discrete scales and certain continuous scales
        channels.iter().all(|c| {
            matches!(
                c.channel_type.as_str(),
                "fill" | "stroke" | "color" | "opacity" | "stroke_width"
            )
        })
    }

    fn supported_merge_channels(&self) -> std::collections::HashSet<&'static str> {
        ["fill", "stroke", "color", "opacity", "stroke_width"]
            .into_iter()
            .collect()
    }

    async fn evaluate(
        &self,
        channels: &[LegendChannel],
        config: &Legend,
        x: f32,
        y: f32,
        _width: f32,
        _height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Option<SceneGroup>, AvengerChartError> {
        if channels.is_empty() {
            return Ok(None);
        }

        // Get the primary channel for this legend
        let primary_channel = &channels[0];

        // Get the domain values
        let domain_values = match primary_channel.scale.domain_values()? {
            DomainValues::Discrete(values) => values,
            DomainValues::Interval(_, _) => {
                // For continuous scales, sample some values
                vec![]
            }
        };

        if domain_values.is_empty() {
            return Ok(None);
        }

        // Get channel name
        let channel_name = &primary_channel.channel_type;

        // Get size value - use constant from mark if available, or use default
        let default_size_value = helpers::get_constant_f32(
            "size",
            &primary_channel.related_channels,
            &self.mark_encodings,
            ctx,
        )
        .unwrap_or(Self::DEFAULT_SIZE);

        // Evaluate title expression
        let title = if let Some(node) = config.title.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            Some(evaluate_string_expr(&expr, ctx, params).await?)
        } else {
            None
        };

        // Evaluate symbol size (from expression, theme, or mark defaults)
        let symbol_size =
            if let Some(node) = config.symbol_size.as_option().and_then(|o| o.as_ref()) {
                // Expression is set - evaluate it
                let expr = node.to_expr(ctx)?;
                evaluate_f32_expr(&expr, ctx, params).await?
            } else {
                // Query from theme legend context
                let legend_ctx = theme.legend_context_with_params(Some("rect"), params.clone());
                theme
                    .query(&legend_ctx, "symbol-size")
                    .and_then(|v| v.as_font_size(params, theme.get_base_font_size(params)))
                    .unwrap_or(default_size_value)
            };

        // Create legend configuration
        let mut legend_config = SymbolLegendConfig {
            title,
            text: ScalarOrArray::new_scalar("".to_string()), // Will be set later
            shape: ScalarOrArray::new_scalar(
                SymbolShape::from_vega_str("square").unwrap_or_default(),
            ), // Always use square for rect marks
            size: ScalarOrArray::new_scalar(symbol_size),
            angle: ScalarOrArray::new_scalar(Self::DEFAULT_ANGLE),
            fill: ScalarOrArray::new_scalar(parse_color_string_strict(Self::DEFAULT_FILL)?),
            stroke: ScalarOrArray::new_scalar(parse_color_string_strict(Self::DEFAULT_STROKE)?),
            stroke_width: Some(Self::DEFAULT_STROKE_WIDTH),
            inner_width: 0.0,
            inner_height: Self::INNER_HEIGHT,
            outer_margin: 0.0,
            text_padding: Self::TEXT_PADDING,
            background_fill: None,
            background_stroke: None,
            background_corner_radius: None,
            background_padding: None,
            title_color: None,
            label_color: None,
            title_font_family: None,
            title_font_size: None,
            title_font_weight: None,
            label_font_family: None,
            label_font_size: None,
            label_font_weight: None,
        };

        // Evaluate and apply legend background styling if provided
        if let Some(node) = config
            .background_padding
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            legend_config.background_padding = Some(evaluate_f32_expr(&expr, ctx, params).await?);
        }
        if let Some(node) = config
            .background_corner_radius
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            legend_config.background_corner_radius =
                Some(evaluate_f32_expr(&expr, ctx, params).await?);
        }
        if let Some(node) = config.background_fill.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let fill_str = evaluate_string_expr(&expr, ctx, params).await?;
            legend_config.background_fill = Some(parse_color_string_strict(&fill_str)?);
        }
        if let Some(node) = config
            .background_stroke
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            let stroke_str = evaluate_string_expr(&expr, ctx, params).await?;
            legend_config.background_stroke = Some(parse_color_string_strict(&stroke_str)?);
        }

        // Evaluate and apply legend colors
        if let Some(node) = config.title_color.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let title_color = evaluate_string_expr(&expr, ctx, params).await?;
            let color = parse_color_string_strict(&title_color)?;
            legend_config.title_color = Some(match color {
                ColorOrGradient::Color(c) => c,
                _ => {
                    return Err(AvengerChartError::InternalError(format!(
                        "Legend title color '{}' parsed to gradient, expected solid color",
                        title_color
                    )));
                }
            });
        }
        if let Some(node) = config.label_color.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let label_color = evaluate_string_expr(&expr, ctx, params).await?;
            let color = parse_color_string_strict(&label_color)?;
            legend_config.label_color = Some(match color {
                ColorOrGradient::Color(c) => c,
                _ => {
                    return Err(AvengerChartError::InternalError(format!(
                        "Legend label color '{}' parsed to gradient, expected solid color",
                        label_color
                    )));
                }
            });
        }

        // Set typography from legend config
        if let Some(node) = config
            .title_font_family
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            legend_config.title_font_family = Some(evaluate_string_expr(&expr, ctx, params).await?);
        }
        if let Some(node) = config.title_font_size.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            legend_config.title_font_size = Some(evaluate_f32_expr(&expr, ctx, params).await?);
        }
        // Override font sizes with params
        let legend_ctx = theme.legend_context_with_params(Some("rect"), params.clone());
        let title_ctx = legend_ctx.child("title");
        let label_ctx = legend_ctx.child("label");

        if let Some(size) = theme.font_size(&title_ctx) {
            legend_config.title_font_size = Some(size);
        }
        if let Some(size) = theme.font_size(&label_ctx) {
            legend_config.label_font_size = Some(size);
        }
        if let Some(node) = config
            .title_font_weight
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            let weight = evaluate_f32_expr(&expr, ctx, params).await?;
            legend_config.title_font_weight = Some(FontWeight::Number(weight));
        }
        if let Some(node) = config
            .label_font_family
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            legend_config.label_font_family = Some(evaluate_string_expr(&expr, ctx, params).await?);
        }
        if let Some(node) = config
            .label_font_weight
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            let weight = evaluate_f32_expr(&expr, ctx, params).await?;
            legend_config.label_font_weight = Some(FontWeight::Number(weight));
        }

        // Use constant values from mark if available (and not the legend channel itself)
        if channel_name != "fill"
            && channel_name != "color"
            && let Some(color) = helpers::get_constant_color(
                "fill",
                &primary_channel.related_channels,
                &self.mark_encodings,
                ctx,
            )
        {
            legend_config.fill = ScalarOrArray::new_scalar(color);
        }

        if channel_name != "stroke"
            && let Some(color) = helpers::get_constant_color(
                "stroke",
                &primary_channel.related_channels,
                &self.mark_encodings,
                ctx,
            )
        {
            legend_config.stroke = ScalarOrArray::new_scalar(color);
        }

        if channel_name != "stroke_width"
            && let Some(width) = helpers::get_constant_f32(
                "stroke_width",
                &primary_channel.related_channels,
                &self.mark_encodings,
                ctx,
            )
        {
            legend_config.stroke_width = Some(width);
        }

        // Apply the scale mapping based on the legend channel
        match channel_name.as_str() {
            "fill" | "color" => {
                // Rect legends always use discrete entries, so use range colors directly
                // For ordinal scales, range_colors() handles wrapping automatically
                let colors = primary_channel.scale.range_colors()?;
                legend_config.fill = ScalarOrArray::new_array(
                    colors.into_iter().map(ColorOrGradient::Color).collect(),
                );
            }
            "stroke" => {
                // Same as fill/color - always use range colors
                let colors = primary_channel.scale.range_colors()?;
                legend_config.stroke = ScalarOrArray::new_array(
                    colors.into_iter().map(ColorOrGradient::Color).collect(),
                );
            }
            "stroke_width" => {
                let widths = primary_channel
                    .scale
                    .scale_scalars_to_numeric(&domain_values)?;
                legend_config.stroke_width = Some(widths[0]); // Single width for rect legend
            }
            _ => {}
        }

        // Create text labels for legend entries
        // Scales that provide legend_entries have custom labels
        let text_values: Vec<String> = if primary_channel
            .scale
            .scale_impl
            .legend_entries(&primary_channel.scale.config)
            .is_some()
        {
            // Get custom labels from the scale
            primary_channel.scale.domain_labels()?
        } else {
            // Use default string conversion
            domain_values
                .iter()
                .map(|v| v.as_scalar_string())
                .collect::<Result<Vec<_>, _>>()?
        };

        // Add text to the legend config
        legend_config.text = ScalarOrArray::new_array(text_values);

        // Create the legend marks
        let mut legend_group = make_symbol_legend(&legend_config)?;

        // Position the legend
        legend_group.origin = [x, y];

        // Set z-index
        legend_group.zindex = Some(10);

        Ok(Some(legend_group))
    }
}
