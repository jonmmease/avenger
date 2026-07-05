//! Symbol legend renderer for discrete channels

use std::collections::HashMap;

use avenger_chart_core::{
    AvengerChartError, ChannelValue, Legend, LegendRenderItem, LegendRenderOutput,
    ScalarValueHelpers, ScaleRange, SerializableScalarMap, Theme, evaluate_f32_expr,
    evaluate_string_expr,
};
use avenger_chart_core::{ConfiguredScaleLegendExt, DefaultLogicalExprNodeExt, DomainValues};
use avenger_color::{ColorOrGradient, parse_color_string_strict};
use avenger_common::{
    types::SymbolShape,
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend_itemized};
use avenger_scales::scales::RangeKind;
use avenger_text::types::FontWeight;
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use super::{LegendChannel, LegendRenderer, helpers, parse_color_or_gradient_strict};

/// Default symbol size for legends
const DEFAULT_SYMBOL_SIZE: f32 = 64.0;

/// Z-index for legend elements (above data but below title)
const LEGEND_ZINDEX: i32 = 10;

/// Symbol legend renderer for discrete channels
#[serde_as]
#[derive(Default, Serialize, Deserialize)]
pub struct CompiledSymbolLegend {
    /// Plot reference for accessing mark data
    /// Map of mark encodings from the plot
    mark_encodings: HashMap<String, ChannelValue>,
    /// Whether the plot has rect marks
    has_rect_mark: bool,
    /// Theme mark defaults for symbols
    #[serde_as(as = "FromInto<SerializableScalarMap>")]
    symbol_defaults: indexmap::IndexMap<String, ScalarValue>,
    /// Theme mark defaults for rects
    #[serde_as(as = "FromInto<SerializableScalarMap>")]
    rect_defaults: indexmap::IndexMap<String, ScalarValue>,
}

impl CompiledSymbolLegend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_rect_mark(&mut self, is_rect: bool) {
        self.has_rect_mark = is_rect;
    }

    /// Helper to extract f32 from ScalarValue (handles both Float32 and Float64)
    fn extract_f32(value: &ScalarValue) -> Option<f32> {
        value
            .as_f32()
            .ok()
            .or_else(|| value.as_f64().ok().map(|f| f as f32))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[typetag::serde]
impl LegendRenderer for CompiledSymbolLegend {
    fn name(&self) -> &'static str {
        "CompiledSymbolLegend"
    }

    fn can_evaluate(&self, channels: &[LegendChannel]) -> bool {
        // Symbol legends work with any scale that has discrete outputs
        // This includes ordinal, threshold, quantize, quantile, etc.
        channels
            .iter()
            .all(|c| c.scale.scale_impl.range_kind() == RangeKind::Discrete)
    }

    fn supported_merge_channels(&self) -> std::collections::HashSet<&'static str> {
        [
            "fill",
            "stroke",
            "color",
            "shape",
            "size",
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
        // Determine if this is a measure call (x=0, y=0) or actual render
        let is_measure = x == 0.0 && y == 0.0;
        let context = if is_measure { "measure" } else { "render" };

        tracing::debug!(
            context = context,
            x = x,
            y = y,
            width = _width,
            height = _height,
            "Symbol legend render called"
        );
        if channels.is_empty() {
            return Ok(None);
        }

        // Get the primary channel (the one we're creating legend for)
        let primary_channel = &channels[0];
        let channel_name = &primary_channel.channel_type;

        // Extract domain values
        let (domain_values, item_values) = match primary_channel.scale.domain_values()? {
            DomainValues::Discrete(values) => {
                tracing::debug!(
                    channel = channel_name.as_str(),
                    scale_type = ?primary_channel.scale.scale_impl.scale_type(),
                    values = ?values,
                    "Symbol legend domain values"
                );
                let item_values = values.iter().map(format_scalar_value).collect::<Vec<_>>();
                (values, Some(item_values))
            }
            DomainValues::Interval(min, max) => (vec![min, max], None),
        };

        if domain_values.is_empty() {
            return Ok(None);
        }

        // Create text labels - use custom labels for scales with legend entries
        let text_values: Vec<String> = if primary_channel
            .scale
            .scale_impl
            .legend_entries(&primary_channel.scale.config)
            .is_some()
        {
            let labels = primary_channel.scale.domain_labels()?;
            tracing::debug!(
                channel = channel_name.as_str(),
                labels = ?labels,
                "Scale with legend entries - using custom labels"
            );
            labels
        } else {
            domain_values.iter().map(format_scalar_value).collect()
        };

        // Get mark defaults from config's theme or use stored defaults
        let mark_defaults = if let Some(ref theme_defaults) = config.theme_mark_defaults {
            if self.has_rect_mark {
                theme_defaults.get("rect").unwrap_or(&self.rect_defaults)
            } else {
                theme_defaults
                    .get("symbol")
                    .unwrap_or(&self.symbol_defaults)
            }
        } else if self.has_rect_mark {
            &self.rect_defaults
        } else {
            &self.symbol_defaults
        };

        // Extract defaults from theme or use fallbacks
        let default_size = mark_defaults
            .get("size")
            .and_then(Self::extract_f32)
            .unwrap_or(DEFAULT_SYMBOL_SIZE);

        let default_shape = if self.has_rect_mark {
            "square".to_string()
        } else {
            mark_defaults
                .get("shape")
                .and_then(|v| match v {
                    ScalarValue::Utf8(Some(s))
                    | ScalarValue::LargeUtf8(Some(s))
                    | ScalarValue::Utf8View(Some(s)) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "circle".to_string())
        };

        let default_angle = mark_defaults
            .get("angle")
            .and_then(Self::extract_f32)
            .unwrap_or(0.0);

        let default_fill = mark_defaults
            .get("fill")
            .and_then(|v| match v {
                ScalarValue::Utf8(Some(s))
                | ScalarValue::LargeUtf8(Some(s))
                | ScalarValue::Utf8View(Some(s)) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "#4682b4".to_string());

        let default_stroke_width = mark_defaults
            .get("stroke_width")
            .and_then(Self::extract_f32)
            .unwrap_or(1.0);

        // If stroke_width is 0, use transparent stroke to avoid hairline rendering
        let default_stroke = if default_stroke_width == 0.0 {
            "transparent".to_string()
        } else {
            mark_defaults
                .get("stroke")
                .and_then(|v| match v {
                    ScalarValue::Utf8(Some(s))
                    | ScalarValue::LargeUtf8(Some(s))
                    | ScalarValue::Utf8View(Some(s)) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "#000000".to_string())
        };

        // Initialize config with defaults
        tracing::debug!(
            channel = channel_name.as_str(),
            text_values = ?text_values,
            default_size = default_size,
            scale_type = ?primary_channel.scale.scale_impl.scale_type(),
            "Creating symbol legend with inner_width: 0.0, inner_height: 100.0, outer_margin: 0.0, text_padding: 2.0"
        );

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
        let label_text_params = avenger_chart_core::scalar_params_for_label_sources_lenient(
            text_values.iter().map(String::as_str),
            config.label_syntax_mode,
            params,
        );

        let mut legend_config = SymbolLegendConfig {
            title,
            text: ScalarOrArray::new_array(text_values.clone()),
            title_syntax_mode: config.title_syntax_mode,
            title_text_params,
            label_syntax_mode: config.label_syntax_mode,
            label_text_params,
            inner_width: 0.0, // Don't offset internally, we'll position the whole group
            inner_height: 100.0, // Will be calculated by legend
            outer_margin: 0.0, // Don't offset legend entries
            text_padding: 2.0, // Consistent padding
            ..Default::default()
        };

        // Evaluate and set text colors from legend config
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

        // Evaluate and set typography from legend config
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
        let legend_type = if self.has_rect_mark {
            Some("rect")
        } else {
            Some("symbol")
        };
        let legend_ctx = theme.legend_context_with_params(legend_type, params.clone());
        let title_ctx = legend_ctx.child("title");
        let label_ctx = legend_ctx.child("label");

        if let Some(size) = theme.font_size(&title_ctx) {
            legend_config.title_font_size = Some(size);
        }
        if let Some(size) = theme.font_size(&label_ctx) {
            legend_config.label_font_size = Some(size);
        }
        // Do not change renderer defaults for label-padding unless tests opt in elsewhere.
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

        // Evaluate and apply legend background styling if provided
        if let Some(node) = config
            .background_padding
            .as_option()
            .and_then(|o| o.as_ref())
        {
            let expr = node.to_expr(ctx)?;
            let pad = evaluate_f32_expr(&expr, ctx, params).await?;
            legend_config.background_padding = Some(pad);
            tracing::trace!(padding = pad, "Symbol legend padding set");
        } else {
            tracing::trace!("Symbol legend padding: None (will use default)");
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

        // Evaluate symbol size (from expression, theme, or mark defaults)
        let symbol_size =
            if let Some(node) = config.symbol_size.as_option().and_then(|o| o.as_ref()) {
                // Expression is set - evaluate it
                let expr = node.to_expr(ctx)?;
                evaluate_f32_expr(&expr, ctx, params).await?
            } else {
                // Query from theme legend context
                let legend_ctx = theme.legend_context_with_params(Some("symbol"), params.clone());
                theme
                    .query(&legend_ctx, "symbol-size")
                    .and_then(|v| v.as_font_size(params, theme.get_base_font_size(params)))
                    .unwrap_or(default_size)
            };

        // Apply each channel's mapping
        // When multiple channels are present, they all vary together

        // Start with defaults
        legend_config.shape = ScalarOrArray::new_scalar(parse_shape(&default_shape)?);
        legend_config.fill =
            ScalarOrArray::new_scalar(parse_color_or_gradient_strict(&default_fill)?);
        legend_config.stroke =
            ScalarOrArray::new_scalar(parse_color_or_gradient_strict(&default_stroke)?);
        legend_config.size = ScalarOrArray::new_scalar(symbol_size);
        legend_config.angle = ScalarOrArray::new_scalar(default_angle);
        legend_config.stroke_width = Some(default_stroke_width);

        // Process each channel
        for channel in channels {
            match channel.channel_type.as_str() {
                "shape" => {
                    // Shape channel - map domain values to shapes
                    let shape_names = {
                        let names = channel.scale.range_strings()?;
                        if !names.is_empty() {
                            names
                        } else {
                            // Fallback to theme's default shape sequence
                            if let Some(range) = theme.get_range_for_channel(
                                &channel.mark_type,
                                "shape",
                                RangeKind::Discrete,
                                Some(domain_values.len()),
                                params,
                            ) {
                                if let ScaleRange::Discrete(scalars) = range {
                                    // Convert SerializableScalar wrappers to strings
                                    scalars
                                        .iter()
                                        .filter_map(|s| match &s.0 {
                                            ScalarValue::Utf8(Some(string))
                                            | ScalarValue::LargeUtf8(Some(string))
                                            | ScalarValue::Utf8View(Some(string)) => {
                                                Some(string.clone())
                                            }
                                            _ => None,
                                        })
                                        .collect()
                                } else {
                                    // Shouldn't happen - shape ranges should be discrete strings
                                    vec!["circle".to_string()]
                                }
                            } else {
                                // Ultimate fallback if theme doesn't provide shapes
                                vec!["circle".to_string()]
                            }
                        }
                    };

                    let shapes: Result<Vec<_>, _> = domain_values
                        .iter()
                        .enumerate()
                        .map(|(i, _)| parse_shape(&shape_names[i % shape_names.len()]))
                        .collect();
                    let shapes = shapes?;

                    tracing::debug!(
                        channel = "shape",
                        shape_names = ?shape_names,
                        domain_values = ?domain_values,
                        text_values = ?text_values,
                        shapes = ?shapes,
                        "Shape legend configuration"
                    );

                    legend_config.shape = ScalarOrArray::new_array(shapes);
                }
                "size" => {
                    // Size channel - map through scale
                    let sizes = channel.scale.scale_scalars_to_numeric(&domain_values)?;
                    legend_config.size = ScalarOrArray::new_array(sizes);
                }
                "fill" | "color" => {
                    // Symbol legends always use discrete entries, so use range colors directly
                    let colors = channel.scale.range_colors()?;
                    legend_config.fill = ScalarOrArray::new_array(
                        colors.into_iter().map(ColorOrGradient::Color).collect(),
                    );
                }
                "stroke" => {
                    // Same as fill/color - always use range colors
                    let colors = channel.scale.range_colors()?;
                    legend_config.stroke = ScalarOrArray::new_array(
                        colors.into_iter().map(ColorOrGradient::Color).collect(),
                    );
                }
                "angle" => {
                    // Angle channel - map through scale
                    let angles = channel.scale.scale_scalars_to_numeric(&domain_values)?;
                    legend_config.angle = ScalarOrArray::new_array(angles);
                }
                "opacity" => {
                    // Opacity channel - map through scale and apply to fill/stroke alpha
                    let opacities = channel.scale.scale_scalars_to_numeric(&domain_values)?;

                    // Apply to fill colors using map
                    let fill_with_opacity = match legend_config.fill.value() {
                        ScalarOrArrayValue::Array(fills) => {
                            let modified: Vec<ColorOrGradient> = fills
                                .iter()
                                .zip(opacities.iter())
                                .map(|(fill, opacity)| {
                                    let mut fill = fill.clone();
                                    if let ColorOrGradient::Color(ref mut color) = fill {
                                        color[3] *= opacity.clamp(0.0, 1.0);
                                    }
                                    fill
                                })
                                .collect();
                            ScalarOrArray::new_array(modified)
                        }
                        ScalarOrArrayValue::Scalar(fill) => {
                            // If fill is scalar, apply first opacity value
                            let mut fill = fill.clone();
                            if let Some(opacity) = opacities.first()
                                && let ColorOrGradient::Color(ref mut color) = fill
                            {
                                color[3] *= opacity.clamp(0.0, 1.0);
                            }
                            ScalarOrArray::new_scalar(fill)
                        }
                    };
                    legend_config.fill = fill_with_opacity;

                    // Apply to stroke colors using map
                    let stroke_with_opacity = match legend_config.stroke.value() {
                        ScalarOrArrayValue::Array(strokes) => {
                            let modified: Vec<ColorOrGradient> = strokes
                                .iter()
                                .zip(opacities.iter())
                                .map(|(stroke, opacity)| {
                                    let mut stroke = stroke.clone();
                                    if let ColorOrGradient::Color(ref mut color) = stroke {
                                        color[3] *= opacity.clamp(0.0, 1.0);
                                    }
                                    stroke
                                })
                                .collect();
                            ScalarOrArray::new_array(modified)
                        }
                        ScalarOrArrayValue::Scalar(stroke) => {
                            // If stroke is scalar, apply first opacity value
                            let mut stroke = stroke.clone();
                            if let Some(opacity) = opacities.first()
                                && let ColorOrGradient::Color(ref mut color) = stroke
                            {
                                color[3] *= opacity.clamp(0.0, 1.0);
                            }
                            ScalarOrArray::new_scalar(stroke)
                        }
                    };
                    legend_config.stroke = stroke_with_opacity;
                }
                "stroke_width" => {
                    // Stroke width channel - map through scale
                    let widths = channel.scale.scale_scalars_to_numeric(&domain_values)?;
                    // SymbolLegendConfig expects a single stroke_width value
                    // Use the first value for now
                    if !widths.is_empty() {
                        legend_config.stroke_width = Some(widths[0]);
                    }
                }
                _ => {
                    // Unknown channel type - skip
                }
            }
        }

        // Apply constant values from related channels if not varying
        // This ensures legends use the same visual properties as the marks

        if channels.iter().all(|c| c.channel_type != "stroke_width") {
            // Stroke width not varying - use constant if available
            if let Some(width) = helpers::get_constant_f32(
                "stroke_width",
                &primary_channel.related_channels,
                &self.mark_encodings,
                ctx,
            ) {
                legend_config.stroke_width = Some(width);
            }
        }

        if channels.iter().all(|c| c.channel_type != "angle") {
            // Angle not varying - use constant if available
            if let Some(angle) = helpers::get_constant_f32(
                "angle",
                &primary_channel.related_channels,
                &self.mark_encodings,
                ctx,
            ) {
                legend_config.angle = ScalarOrArray::new_scalar(angle);
            }
        }

        if channels.iter().all(|c| c.channel_type != "shape") {
            // Shape not varying - use constant if available
            if let Some(shape_str) = helpers::get_constant_string(
                "shape",
                &primary_channel.related_channels,
                &self.mark_encodings,
                ctx,
            ) {
                legend_config.shape = ScalarOrArray::new_scalar(parse_shape(&shape_str)?);
            }
        }

        if channels
            .iter()
            .all(|c| c.channel_type != "fill" && c.channel_type != "color")
        {
            // Fill not varying - use constant if available
            if let Some(color) = helpers::get_constant_color(
                "fill",
                &primary_channel.related_channels,
                &self.mark_encodings,
                ctx,
            ) {
                legend_config.fill = ScalarOrArray::new_scalar(color);
            }
        }

        if channels.iter().all(|c| c.channel_type != "stroke") {
            // Stroke not varying - use constant if available
            if let Some(color) = helpers::get_constant_color(
                "stroke",
                &primary_channel.related_channels,
                &self.mark_encodings,
                ctx,
            ) {
                legend_config.stroke = ScalarOrArray::new_scalar(color);
            }
        }

        if channels.iter().all(|c| c.channel_type != "size") {
            // Size not varying - use constant if available
            tracing::trace!(
                related_channels = ?primary_channel.related_channels.keys().collect::<Vec<_>>(),
                mark_encodings = ?self.mark_encodings.keys().collect::<Vec<_>>(),
                default_size = default_size,
                "Checking for constant size in legend"
            );

            if let Some(size_value) = helpers::get_constant_f32(
                "size",
                &primary_channel.related_channels,
                &self.mark_encodings,
                ctx,
            ) {
                tracing::trace!(size_value = size_value, "Found constant size");
                legend_config.size = ScalarOrArray::new_scalar(size_value);
            } else {
                // No explicit size channel - use theme default if it's not the standard default
                tracing::trace!(
                    default_size = default_size,
                    standard_size = DEFAULT_SYMBOL_SIZE,
                    "No constant size found, checking theme default"
                );
                // If the theme has set a non-standard size, use it
                if default_size != DEFAULT_SYMBOL_SIZE {
                    tracing::trace!(default_size = default_size, "Using theme default size");
                    legend_config.size = ScalarOrArray::new_scalar(default_size);
                }
            }
        }

        // Log final legend configuration before rendering
        tracing::debug!(
            title = ?legend_config.title,
            text = ?legend_config.text,
            shape = ?legend_config.shape,
            size = ?legend_config.size,
            x = x,
            y = y,
            render_context = context,
            "Final symbol legend configuration before rendering"
        );

        // Create the legend marks
        let mut output = make_symbol_legend_itemized(&legend_config)?;

        // Position the legend
        output.group.origin = [x, y];
        output.group.zindex = Some(LEGEND_ZINDEX);
        let items = item_values
            .map(|values| {
                output
                    .items
                    .iter()
                    .zip(values)
                    .map(|(item, value)| LegendRenderItem {
                        index: item.index,
                        label: item.label.clone(),
                        channel: primary_channel.channel_type.clone(),
                        name: item.label.clone(),
                        value,
                        group_path: item.group_path.clone(),
                        hit_rect_path: item.hit_rect_path.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(Some(LegendRenderOutput {
            group: output.group,
            items,
            continuous_surfaces: Vec::new(),
        }))
    }
}

fn format_scalar_value(value: &ScalarValue) -> String {
    match value {
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => s.clone(),
        ScalarValue::Float64(Some(f)) => {
            // Format float nicely - remove trailing zeros
            if f.fract() == 0.0 && f.abs() < 1e10 {
                format!("{:.0}", f)
            } else {
                format!("{}", f)
            }
        }
        ScalarValue::Float32(Some(f)) => {
            if f.fract() == 0.0 && f.abs() < 1e10 {
                format!("{:.0}", f)
            } else {
                format!("{}", f)
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
        _ => format!("{:?}", value), // Fallback for other types
    }
}

/// Parse a shape string into a SymbolShape
fn parse_shape(shape: &str) -> Result<SymbolShape, AvengerChartError> {
    SymbolShape::from_vega_str(shape)
        .map_err(|e| AvengerChartError::InternalError(format!("Invalid shape '{}': {}", shape, e)))
}
