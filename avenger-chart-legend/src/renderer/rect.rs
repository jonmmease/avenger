//! Rectangle/bar legend renderer for discrete scales

use std::collections::HashMap;

use avenger_chart_core::{
    AvengerChartError, ChannelValue, Legend, LegendRenderItem, LegendRenderOutput,
    ScalarValueHelpers, Theme, evaluate_f32_expr, evaluate_string_expr,
};
use avenger_chart_core::{ConfiguredScaleLegendExt, DefaultLogicalExprNodeExt, DomainValues};
use avenger_color::{ColorOrGradient, parse_color_string_strict};
use avenger_common::{types::SymbolShape, value::ScalarOrArray};
use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend_itemized};
use avenger_scenegraph::marks::pattern::{
    PatternAnchor, PatternFill, PatternLayer, default_no_fill_pattern,
};
use avenger_text::types::FontWeight;
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{ChannelInfo, LegendChannel, LegendRenderer, helpers, parse_color_or_gradient_strict};

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
    const DEFAULT_PATTERN_FILL: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    const DEFAULT_STROKE: &'static str = "#000000"; // Black
    const DEFAULT_STROKE_WIDTH: f32 = 1.0;
    const DEFAULT_PATTERN_LEGEND_SIDE: f32 = 32.0;
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
                "fill" | "fill_pattern" | "stroke" | "color" | "opacity" | "stroke_width"
            )
        })
    }

    fn supported_merge_channels(&self) -> std::collections::HashSet<&'static str> {
        [
            "fill",
            "fill_pattern",
            "stroke",
            "color",
            "opacity",
            "stroke_width",
        ]
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
    ) -> Result<Option<LegendRenderOutput>, AvengerChartError> {
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
        let pattern_range = pattern_range_for_channels(channels);
        let has_pattern_range = pattern_range.is_some();

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
        let title_text_params = if let Some(title_text) = title.as_deref() {
            avenger_chart_core::scalar_params_for_label_source(
                title_text,
                config.title_syntax_mode,
                params,
            )?
        } else {
            avenger_text::LabelParams::default()
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
                if has_pattern_range {
                    let base_font_size = theme.get_base_font_size(params);
                    let side = theme
                        .query(&legend_ctx, "pattern-legend-size")
                        .and_then(|v| v.as_font_size(params, base_font_size))
                        .unwrap_or(Self::DEFAULT_PATTERN_LEGEND_SIDE);
                    side * side
                } else {
                    theme
                        .query(&legend_ctx, "symbol-size")
                        .and_then(|v| v.as_font_size(params, theme.get_base_font_size(params)))
                        .unwrap_or(default_size_value)
                }
            };
        let sample_side = symbol_size.sqrt();

        // Create legend configuration
        let mut legend_config = SymbolLegendConfig {
            title,
            text: ScalarOrArray::new_scalar("".to_string()), // Will be set later
            title_syntax_mode: config.title_syntax_mode,
            title_text_params,
            label_syntax_mode: config.label_syntax_mode,
            label_text_params: avenger_text::LabelParams::default(),
            shape: ScalarOrArray::new_scalar(
                SymbolShape::from_vega_str("square").unwrap_or_default(),
            ), // Always use square for rect marks
            size: ScalarOrArray::new_scalar(symbol_size),
            angle: ScalarOrArray::new_scalar(Self::DEFAULT_ANGLE),
            fill: ScalarOrArray::new_scalar(parse_color_or_gradient_strict(Self::DEFAULT_FILL)?),
            fill_pattern: default_no_fill_pattern(),
            stroke: ScalarOrArray::new_scalar(parse_color_or_gradient_strict(
                Self::DEFAULT_STROKE,
            )?),
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
            legend_config.background_fill = Some(parse_color_or_gradient_strict(&fill_str)?);
        }
        if let Some(node) = config
            .background_stroke
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            let stroke_str = evaluate_string_expr(&expr, ctx, params).await?;
            legend_config.background_stroke = Some(parse_color_or_gradient_strict(&stroke_str)?);
        }

        // Evaluate and apply legend colors
        if let Some(node) = config.title_color.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let title_color = evaluate_string_expr(&expr, ctx, params).await?;
            legend_config.title_color = Some(parse_color_string_strict(&title_color)?);
        }
        if let Some(node) = config.label_color.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            let label_color = evaluate_string_expr(&expr, ctx, params).await?;
            legend_config.label_color = Some(parse_color_string_strict(&label_color)?);
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
            "fill_pattern" => {
                if !channels
                    .iter()
                    .any(|channel| matches!(channel.channel_type.as_str(), "fill" | "color"))
                {
                    legend_config.fill = ScalarOrArray::new_scalar(ColorOrGradient::Color(
                        Self::DEFAULT_PATTERN_FILL,
                    ));
                }
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

        if let Some(pattern_range) = pattern_range {
            if !pattern_range.is_empty() {
                legend_config.fill_pattern = ScalarOrArray::new_array(
                    (0..domain_values.len())
                        .map(|index| {
                            pattern_range[index % pattern_range.len()]
                                .as_ref()
                                .map(|pattern| centered_legend_pattern(pattern, sample_side))
                        })
                        .collect(),
                );
            }
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
        legend_config.label_text_params =
            avenger_chart_core::scalar_params_for_label_sources_lenient(
                text_values.iter().map(String::as_str),
                config.label_syntax_mode,
                params,
            );

        // Add text to the legend config
        legend_config.text = ScalarOrArray::new_array(text_values);

        // Create the legend marks
        let mut output = make_symbol_legend_itemized(&legend_config)?;

        // Position the legend
        output.group.origin = [x, y];

        // Set z-index
        output.group.zindex = Some(10);
        let mut items = Vec::new();
        for (item, value) in output.items.iter().zip(domain_values.iter()) {
            items.push(LegendRenderItem {
                index: item.index,
                label: item.label.clone(),
                channel: primary_channel.channel_type.clone(),
                value: format_scalar_value(value),
                name: item.label.clone(),
                group_path: item.group_path.clone(),
                hit_rect_path: item.hit_rect_path.clone(),
            });
        }

        Ok(Some(LegendRenderOutput {
            group: output.group,
            items,
            continuous_surfaces: Vec::new(),
        }))
    }
}

fn pattern_range_for_channels(channels: &[LegendChannel]) -> Option<&[Option<PatternFill>]> {
    for channel in channels {
        if channel.channel_type == "fill_pattern"
            && let Some(range) = channel.pattern_range.as_deref()
        {
            return Some(range);
        }
    }

    for channel in channels {
        if let Some(ChannelInfo::Scaled {
            pattern_range: Some(range),
            ..
        }) = channel.related_channels.get("fill_pattern")
        {
            return Some(range);
        }
    }

    None
}

fn centered_legend_pattern(pattern: &PatternFill, sample_side: f32) -> PatternFill {
    let mut pattern = pattern.clone();
    pattern.anchor = PatternAnchor::Mark;
    let center = [sample_side / 2.0, sample_side / 2.0];

    for layer in &mut pattern.layers {
        match layer {
            PatternLayer::Stripe(stripe) => {
                let theta = stripe.angle.to_radians();
                let normal = [-theta.sin(), theta.cos()];
                stripe.phase += normal[0] * center[0] + normal[1] * center[1];
            }
            PatternLayer::Symbol(symbol) => {
                symbol.lattice.u_phase += center[0];
                symbol.lattice.v_phase += center[1];
            }
        }
    }

    pattern
}

fn format_scalar_value(value: &ScalarValue) -> String {
    match value {
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => s.clone(),
        ScalarValue::Float64(Some(f)) => {
            if f.fract() == 0.0 && f.abs() < 1e10 {
                format!("{f:.0}")
            } else {
                f.to_string()
            }
        }
        ScalarValue::Float32(Some(f)) => {
            if f.fract() == 0.0 && f.abs() < 1e10 {
                format!("{f:.0}")
            } else {
                f.to_string()
            }
        }
        ScalarValue::Int64(Some(i)) => i.to_string(),
        ScalarValue::Int32(Some(i)) => i.to_string(),
        ScalarValue::Int16(Some(i)) => i.to_string(),
        ScalarValue::Int8(Some(i)) => i.to_string(),
        ScalarValue::UInt64(Some(i)) => i.to_string(),
        ScalarValue::UInt32(Some(i)) => i.to_string(),
        ScalarValue::UInt16(Some(i)) => i.to_string(),
        ScalarValue::UInt8(Some(i)) => i.to_string(),
        _ => format!("{value:?}"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use avenger_chart_core::LegendRenderer;
    use avenger_common::value::ScalarOrArrayValue;
    use avenger_scales::scales::{ConfiguredScale, band::BandScale, ordinal::OrdinalScale};
    use avenger_scenegraph::marks::{mark::SceneMark, pattern::StripePatternLayer};
    use datafusion::common::ScalarValue;
    use datafusion::prelude::SessionContext;

    use super::*;

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn make_simple_scale() -> ConfiguredScale {
        let domain = ScalarValue::iter_to_array(vec![s("A"), s("B")]).unwrap();
        BandScale::configured(domain, (0.0, 100.0))
    }

    fn make_color_scale() -> ConfiguredScale {
        let domain = ScalarValue::iter_to_array(vec![s("A"), s("B")]).unwrap();
        let range = ScalarValue::iter_to_array(vec![s("#000000"), s("#ffffff")]).unwrap();
        OrdinalScale::configured(domain).with_range(range)
    }

    fn stripe_pattern() -> PatternFill {
        PatternFill {
            layers: vec![PatternLayer::Stripe(StripePatternLayer::new(
                0.0, 16.0, 2.0,
            ))],
            ..Default::default()
        }
    }

    #[test]
    fn rect_legend_renders_centered_pattern_samples() {
        futures::executor::block_on(async {
            let pattern = stripe_pattern();
            let channel = LegendChannel {
                name: "fill_pattern".to_string(),
                expression: None,
                scale: make_simple_scale(),
                channel_type: "fill_pattern".to_string(),
                sharing_level: None,
                mark_type: "rect".to_string(),
                mark_index: 0,
                pattern_range: Some(vec![Some(pattern), None]),
                related_channels: HashMap::new(),
            };
            let theme = Theme::from_css(
                r#"
                    legend[type="rect"] {
                        pattern-legend-size: 24px;
                    }
                "#,
            )
            .unwrap();
            let output = CompiledRectLegend::new()
                .evaluate(
                    &[channel],
                    &Legend::new(),
                    0.0,
                    0.0,
                    100.0,
                    100.0,
                    &theme,
                    &IndexMap::new(),
                    &SessionContext::new(),
                )
                .await
                .unwrap()
                .expect("legend output");

            let SceneMark::Group(first_item) = &output.group.marks[1] else {
                panic!("first legend item should be a group");
            };
            let SceneMark::Symbol(symbol) = &first_item.marks[1] else {
                panic!("first legend item should contain a symbol mark");
            };

            assert_eq!(symbol.size_vec(), vec![24.0 * 24.0]);
            assert_eq!(
                symbol.fill_vec(),
                vec![ColorOrGradient::Color(
                    CompiledRectLegend::DEFAULT_PATTERN_FILL
                )]
            );
            let patterns = symbol.fill_pattern_vec();
            let pattern = patterns[0].as_ref().expect("first item has pattern");
            assert_eq!(pattern.anchor, PatternAnchor::Mark);
            let PatternLayer::Stripe(stripe) = &pattern.layers[0] else {
                panic!("expected centered stripe layer");
            };
            assert_eq!(stripe.phase, 12.0);

            let SceneMark::Group(second_item) = &output.group.marks[2] else {
                panic!("second legend item should be a group");
            };
            let SceneMark::Symbol(symbol) = &second_item.marks[1] else {
                panic!("second legend item should contain a symbol mark");
            };
            assert!(matches!(
                symbol.fill_pattern.value(),
                ScalarOrArrayValue::Array(values) if values[1].is_none()
            ));
        });
    }

    #[test]
    fn rect_legend_can_merge_fill_and_fill_pattern_channels() {
        let renderer = CompiledRectLegend::new();
        let channels = vec![
            LegendChannel {
                name: "fill".to_string(),
                expression: None,
                scale: make_simple_scale(),
                channel_type: "fill".to_string(),
                sharing_level: None,
                mark_type: "rect".to_string(),
                mark_index: 0,
                pattern_range: None,
                related_channels: HashMap::new(),
            },
            LegendChannel {
                name: "fill_pattern".to_string(),
                expression: None,
                scale: make_simple_scale(),
                channel_type: "fill_pattern".to_string(),
                sharing_level: None,
                mark_type: "rect".to_string(),
                mark_index: 0,
                pattern_range: Some(vec![Some(stripe_pattern())]),
                related_channels: HashMap::new(),
            },
        ];

        assert!(renderer.can_evaluate(&channels));
        assert!(renderer.supports_merge(&channels));
    }

    #[test]
    fn rect_legend_renders_merged_fill_and_pattern_samples() {
        futures::executor::block_on(async {
            let pattern = stripe_pattern();
            let channels = vec![
                LegendChannel {
                    name: "fill".to_string(),
                    expression: None,
                    scale: make_color_scale(),
                    channel_type: "fill".to_string(),
                    sharing_level: None,
                    mark_type: "rect".to_string(),
                    mark_index: 0,
                    pattern_range: None,
                    related_channels: HashMap::new(),
                },
                LegendChannel {
                    name: "fill_pattern".to_string(),
                    expression: None,
                    scale: make_simple_scale(),
                    channel_type: "fill_pattern".to_string(),
                    sharing_level: None,
                    mark_type: "rect".to_string(),
                    mark_index: 0,
                    pattern_range: Some(vec![Some(pattern), None]),
                    related_channels: HashMap::new(),
                },
            ];

            let output = CompiledRectLegend::new()
                .evaluate(
                    &channels,
                    &Legend::new(),
                    0.0,
                    0.0,
                    100.0,
                    100.0,
                    &Theme::light(),
                    &IndexMap::new(),
                    &SessionContext::new(),
                )
                .await
                .unwrap()
                .expect("legend output");

            let SceneMark::Group(first_item) = &output.group.marks[1] else {
                panic!("first legend item should be a group");
            };
            let SceneMark::Symbol(symbol) = &first_item.marks[1] else {
                panic!("first legend item should contain a symbol mark");
            };

            assert_eq!(
                symbol.fill_vec(),
                vec![ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])]
            );
            assert_eq!(
                symbol.size_vec(),
                vec![CompiledRectLegend::DEFAULT_PATTERN_LEGEND_SIDE.powi(2)]
            );
            let patterns = symbol.fill_pattern_vec();
            let pattern = patterns[0].as_ref().expect("first item has pattern");
            assert_eq!(pattern.anchor, PatternAnchor::Mark);
            let PatternLayer::Stripe(stripe) = &pattern.layers[0] else {
                panic!("expected centered stripe layer");
            };
            assert_eq!(
                stripe.phase,
                CompiledRectLegend::DEFAULT_PATTERN_LEGEND_SIDE / 2.0
            );

            let SceneMark::Group(second_item) = &output.group.marks[2] else {
                panic!("second legend item should be a group");
            };
            let SceneMark::Symbol(symbol) = &second_item.marks[1] else {
                panic!("second legend item should contain a symbol mark");
            };
            assert_eq!(
                symbol.fill_vec(),
                vec![ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0])]
            );
            assert!(matches!(
                symbol.fill_pattern.value(),
                ScalarOrArrayValue::Array(values) if values[1].is_none()
            ));
        });
    }
}
