//! Symbol legend renderer for discrete channels

use super::{LegendChannel, LegendRenderer, helpers};
use crate::error::AvengerChartError;
use crate::legend::Legend;
use crate::scales::{ConfiguredScaleLegendExt, DomainValues};
use crate::serialization::SerializableScalarMap;
use crate::utils::ScalarValueHelpers;
use avenger_common::types::{ColorOrGradient, SymbolShape};
use avenger_common::value::ScalarOrArray;
use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend};
use avenger_scenegraph::marks::group::SceneGroup;
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::collections::HashMap;

/// Symbol legend renderer for discrete channels
#[serde_as]
#[derive(Default, Serialize, Deserialize)]
pub struct CompiledSymbolLegend {
    /// Plot reference for accessing mark data
    /// Map of mark encodings from the plot
    mark_encodings: HashMap<String, crate::channel::ChannelValue>,
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
}

#[typetag::serde]
impl LegendRenderer for CompiledSymbolLegend {
    fn name(&self) -> &'static str {
        "CompiledSymbolLegend"
    }

    fn can_render(&self, channels: &[LegendChannel]) -> bool {
        use avenger_scales::scales::RangeKind;

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

    fn render(
        &self,
        channels: &[LegendChannel],
        config: &Legend,
        x: f32,
        y: f32,
        _width: f32,
        _height: f32,
        theme: &crate::theme::Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Option<SceneGroup>, AvengerChartError> {
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
        let domain_values = match primary_channel.scale.domain_values()? {
            DomainValues::Discrete(values) => {
                tracing::debug!(
                    channel = channel_name.as_str(),
                    scale_type = ?primary_channel.scale.scale_impl.scale_type(),
                    values = ?values,
                    "Symbol legend domain values"
                );
                values
            }
            DomainValues::Interval(min, max) => vec![min, max],
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
            .and_then(|v| {
                // Try both Float32 and Float64
                v.as_f32()
                    .ok()
                    .or_else(|| v.as_f64().ok().map(|f| f as f32))
            })
            .unwrap_or(64.0);

        let default_shape = if self.has_rect_mark {
            "square".to_string()
        } else {
            mark_defaults
                .get("shape")
                .and_then(|v| match v {
                    ScalarValue::Utf8(Some(s)) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "circle".to_string())
        };

        let default_angle = mark_defaults
            .get("angle")
            .and_then(|v| {
                // Try both Float32 and Float64
                v.as_f32()
                    .ok()
                    .or_else(|| v.as_f64().ok().map(|f| f as f32))
            })
            .unwrap_or(0.0);

        let default_fill = mark_defaults
            .get("fill")
            .and_then(|v| match v {
                ScalarValue::Utf8(Some(s)) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "#4682b4".to_string());

        let default_stroke_width = mark_defaults
            .get("stroke_width")
            .and_then(|v| {
                // Try both Float32 and Float64
                v.as_f32()
                    .ok()
                    .or_else(|| v.as_f64().ok().map(|f| f as f32))
            })
            .unwrap_or(1.0);

        // If stroke_width is 0, use transparent stroke to avoid hairline rendering
        let default_stroke = if default_stroke_width == 0.0 {
            "transparent".to_string()
        } else {
            mark_defaults
                .get("stroke")
                .and_then(|v| match v {
                    ScalarValue::Utf8(Some(s)) => Some(s.clone()),
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

        let mut legend_config = SymbolLegendConfig {
            title: config.title.clone().into_option(),
            text: ScalarOrArray::new_array(text_values.clone()),
            inner_width: 0.0, // Don't offset internally, we'll position the whole group
            inner_height: 100.0, // Will be calculated by legend
            outer_margin: 0.0, // Don't offset legend entries
            text_padding: 2.0, // Consistent padding
            ..Default::default()
        };

        // Set text colors from legend config - fail if colors cannot be parsed
        if let Some(title_color) = config.title_color.as_option() {
            let color = crate::utils::parse_color_string_strict(title_color)?;
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
        if let Some(label_color) = config.label_color.as_option() {
            let color = crate::utils::parse_color_string_strict(label_color)?;
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
        if let Some(family) = config.title_font_family.as_option() {
            legend_config.title_font_family = Some(family.clone());
        }
        if let Some(size) = config.title_font_size.as_option() {
            legend_config.title_font_size = Some(*size);
        }
        // Override font sizes with params
        let legend_type = if self.has_rect_mark {
            Some("rect")
        } else {
            Some("symbol")
        };
        let legend_ctx = theme
            .legend_context(legend_type)
            .with_params(params.clone());
        let title_ctx = legend_ctx.child("title");
        let label_ctx = legend_ctx.child("label");

        if let Some(size) = theme.font_size(&title_ctx) {
            legend_config.title_font_size = Some(size);
        }
        if let Some(size) = theme.font_size(&label_ctx) {
            legend_config.label_font_size = Some(size);
        }
        if let Some(weight) = config.title_font_weight.as_option() {
            legend_config.title_font_weight =
                Some(avenger_text::types::FontWeight::Number(*weight));
        }
        if let Some(family) = config.label_font_family.as_option() {
            legend_config.label_font_family = Some(family.clone());
        }
        if let Some(weight) = config.label_font_weight.as_option() {
            legend_config.label_font_weight =
                Some(avenger_text::types::FontWeight::Number(*weight));
        }

        // Apply legend background styling if provided
        if let Some(pad) = config.background_padding.as_option() {
            legend_config.background_padding = Some(*pad);
            tracing::trace!(padding = pad, "Symbol legend padding set");
        } else {
            tracing::trace!("Symbol legend padding: None (will use default)");
        }
        if let Some(r) = config.background_corner_radius.as_option() {
            legend_config.background_corner_radius = Some(*r);
        }
        if let Some(fill_str) = config.background_fill.as_option() {
            legend_config.background_fill =
                Some(crate::utils::parse_color_string_strict(fill_str)?);
        }
        if let Some(stroke_str) = config.background_stroke.as_option() {
            legend_config.background_stroke =
                Some(crate::utils::parse_color_string_strict(stroke_str)?);
        }

        // Apply each channel's mapping
        // When multiple channels are present, they all vary together

        // Start with defaults
        legend_config.shape = ScalarOrArray::new_scalar(parse_shape(&default_shape)?);
        legend_config.fill =
            ScalarOrArray::new_scalar(crate::utils::parse_color_string_strict(&default_fill)?);
        legend_config.stroke =
            ScalarOrArray::new_scalar(crate::utils::parse_color_string_strict(&default_stroke)?);
        legend_config.size = ScalarOrArray::new_scalar(default_size);
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
                            // Fallback to a default shape sequence if scale doesn't provide shapes
                            // This shouldn't happen if scales are properly configured with theme
                            vec![
                                "circle".to_string(),
                                "cross".to_string(),
                                "diamond".to_string(),
                                "square".to_string(),
                                "star".to_string(),
                                "triangle-up".to_string(),
                                "wye".to_string(),
                                "cushion".to_string(),
                            ]
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
                    // Opacity channel - map through scale
                    let _opacities = channel.scale.scale_scalars_to_numeric(&domain_values)?;
                    // TODO: Apply opacity to fill/stroke colors
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
            // Create a temporary SessionContext for get_constant_f32
            let session_context = datafusion::prelude::SessionContext::new();
            if let Some(width) = helpers::get_constant_f32(
                "stroke_width",
                &primary_channel.related_channels,
                &self.mark_encodings,
                &session_context,
            ) {
                legend_config.stroke_width = Some(width);
            }
        }

        if channels.iter().all(|c| c.channel_type != "angle") {
            // Angle not varying - use constant if available
            // Create a temporary SessionContext for get_constant_f32
            let session_context = datafusion::prelude::SessionContext::new();
            if let Some(angle) = helpers::get_constant_f32(
                "angle",
                &primary_channel.related_channels,
                &self.mark_encodings,
                &session_context,
            ) {
                legend_config.angle = ScalarOrArray::new_scalar(angle);
            }
        }

        if channels.iter().all(|c| c.channel_type != "shape") {
            // Shape not varying - use constant if available
            // Create a temporary SessionContext for get_constant_string
            let session_context = datafusion::prelude::SessionContext::new();
            if let Some(shape_str) = helpers::get_constant_string(
                "shape",
                &primary_channel.related_channels,
                &self.mark_encodings,
                &session_context,
            ) {
                legend_config.shape = ScalarOrArray::new_scalar(parse_shape(&shape_str)?);
            }
        }

        if channels
            .iter()
            .all(|c| c.channel_type != "fill" && c.channel_type != "color")
        {
            // Fill not varying - use constant if available
            // Create a temporary SessionContext for get_constant_color
            let session_context = datafusion::prelude::SessionContext::new();
            if let Some(color) = helpers::get_constant_color(
                "fill",
                &primary_channel.related_channels,
                &self.mark_encodings,
                &session_context,
            ) {
                legend_config.fill = ScalarOrArray::new_scalar(color);
            }
        }

        if channels.iter().all(|c| c.channel_type != "stroke") {
            // Stroke not varying - use constant if available
            // Create a temporary SessionContext for get_constant_color
            let session_context = datafusion::prelude::SessionContext::new();
            if let Some(color) = helpers::get_constant_color(
                "stroke",
                &primary_channel.related_channels,
                &self.mark_encodings,
                &session_context,
            ) {
                legend_config.stroke = ScalarOrArray::new_scalar(color);
            }
        }

        if channels.iter().all(|c| c.channel_type != "size") {
            // Size not varying - use constant if available
            if std::env::var("AVENGER_DEBUG_LEGEND").is_ok() {
                eprintln!("DEBUG: Checking for constant size in legend");
                eprintln!(
                    "  related_channels keys: {:?}",
                    primary_channel.related_channels.keys().collect::<Vec<_>>()
                );
                eprintln!(
                    "  mark_encodings keys: {:?}",
                    self.mark_encodings.keys().collect::<Vec<_>>()
                );
                eprintln!("  default_size from theme: {}", default_size);
            }

            // Create a temporary SessionContext for get_constant_f32
            let session_context = datafusion::prelude::SessionContext::new();
            if let Some(size_value) = helpers::get_constant_f32(
                "size",
                &primary_channel.related_channels,
                &self.mark_encodings,
                &session_context,
            ) {
                if std::env::var("AVENGER_DEBUG_LEGEND").is_ok() {
                    eprintln!("  Found constant size: {}", size_value);
                }
                legend_config.size = ScalarOrArray::new_scalar(size_value);
            } else {
                // No explicit size channel - use theme default if it's not the standard default
                if std::env::var("AVENGER_DEBUG_LEGEND").is_ok() {
                    eprintln!(
                        "  No constant size found, checking if theme default {} is different from standard 64.0",
                        default_size
                    );
                }
                // If the theme has set a non-standard size, use it
                if default_size != 64.0 {
                    if std::env::var("AVENGER_DEBUG_LEGEND").is_ok() {
                        eprintln!("  Using theme default size: {}", default_size);
                    }
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
        let mut legend_group = make_symbol_legend(&legend_config)?;

        // Position the legend
        legend_group.origin = [x, y];
        legend_group.zindex = Some(10); // Legends above data but below title

        Ok(Some(legend_group))
    }
}

fn format_scalar_value(value: &ScalarValue) -> String {
    match value {
        ScalarValue::Utf8(Some(s)) => s.clone(),
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
