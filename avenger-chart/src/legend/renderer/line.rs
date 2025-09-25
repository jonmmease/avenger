//! Line legend renderer for stroke properties on line marks

use super::{LegendChannel, LegendRenderer, helpers};
use crate::error::AvengerChartError;
use crate::legend::Legend;
use crate::scales::{ConfiguredScaleLegendExt, DomainValues};
use avenger_common::types::{ColorOrGradient, StrokeCap, StrokeJoin};
use avenger_common::value::ScalarOrArray;
use avenger_guides::legend::line::{LineLegendConfig, make_line_legend};
use avenger_scenegraph::marks::group::SceneGroup;
use datafusion::arrow::array::{ArrayRef, StringArray};
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Line legend renderer for stroke properties on line marks
#[derive(Serialize, Deserialize)]
pub struct LineLegendRenderer {
    /// Map of mark encodings from the plot
    #[serde(skip)]
    mark_encodings: HashMap<String, crate::channel::ChannelValue>,
    /// Stroke cap and join settings from line marks
    stroke_cap: StrokeCap,
    stroke_join: StrokeJoin,
}

impl Default for LineLegendRenderer {
    fn default() -> Self {
        Self {
            mark_encodings: HashMap::new(),
            stroke_cap: StrokeCap::Round,
            stroke_join: StrokeJoin::Round,
        }
    }
}

impl LineLegendRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Convert dash pattern names to numeric arrays using the coercer
    fn convert_dash_pattern(pattern: &str) -> Option<Vec<f32>> {
        use avenger_scales::scales::coerce::Coercer;

        // Create a single-element string array with the pattern
        let array = StringArray::from(vec![Some(pattern)]);
        let array_ref = Arc::new(array) as ArrayRef;

        // Use coercer to convert
        let coercer = Coercer::default();
        if let Ok(dash_result) = coercer.to_stroke_dash(&array_ref) {
            // Get the first element from the ScalarOrArray result
            if let Some(dash_vec) = dash_result.first() {
                if dash_vec.is_empty() {
                    None // solid pattern
                } else {
                    Some(dash_vec.clone())
                }
            } else {
                None
            }
        } else {
            None
        }
    }
}

#[typetag::serde]
impl LegendRenderer for LineLegendRenderer {
    fn name(&self) -> &'static str {
        "LineLegendRenderer"
    }

    fn can_render(&self, channels: &[LegendChannel]) -> bool {
        // Line legend is for line marks with stroke properties
        channels.iter().any(|c| c.mark_type == "line")
            && channels.iter().all(|c| {
                matches!(
                    c.channel_type.as_str(),
                    "stroke" | "stroke_width" | "stroke_dash" | "stroke_opacity"
                )
            })
    }

    fn supported_merge_channels(&self) -> std::collections::HashSet<&'static str> {
        ["stroke", "stroke_width", "stroke_dash", "opacity"]
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
    ) -> Result<Option<SceneGroup>, AvengerChartError> {
        if channels.is_empty() {
            return Ok(None);
        }

        // Get the primary channel
        let primary_channel = &channels[0];
        let channel_name = &primary_channel.channel_type;

        // Extract domain values
        let domain_values = match primary_channel.scale.domain_values()? {
            DomainValues::Discrete(values) => values,
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

        // Get default stroke properties
        let _default_stroke = "#000000".to_string();
        let default_stroke_width = 2.0;

        // Initialize config with defaults
        // Use longer line length for better dash pattern visibility
        let mut legend_config = LineLegendConfig {
            title: config.title.clone().into_option(),
            text: ScalarOrArray::new_array(text_values),
            stroke_cap: self.stroke_cap,
            stroke_join: Some(self.stroke_join), // Add stroke_join to config
            inner_width: 0.0,
            inner_height: 100.0,
            outer_margin: 0.0, // Don't offset legend entries
            line_length: ScalarOrArray::new_scalar(16.0), // Default, will be adjusted for dash patterns
            text_padding: 4.0,                            // Consistent with symbol legend
            ..Default::default()
        };

        // Apply legend background styling if provided
        if let Some(pad) = config.background_padding.as_option() {
            legend_config.background_padding = Some(*pad);
            tracing::trace!(
                channel = channel_name.as_str(),
                padding = pad,
                "Line legend setting padding"
            );
        } else {
            tracing::trace!(
                channel = channel_name.as_str(),
                "Line legend has no padding specified, will use default"
            );
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
        if let Some(weight) = config.title_font_weight.as_option() {
            legend_config.title_font_weight =
                Some(avenger_text::types::FontWeight::Number(*weight));
        }
        if let Some(family) = config.label_font_family.as_option() {
            legend_config.label_font_family = Some(family.clone());
        }
        if let Some(size) = config.label_font_size.as_option() {
            legend_config.label_font_size = Some(*size);
        }
        if let Some(weight) = config.label_font_weight.as_option() {
            legend_config.label_font_weight =
                Some(avenger_text::types::FontWeight::Number(*weight));
        }

        // When multiple channels are present, vary all of them together
        // Check each channel in the group to see if it should vary

        // Set stroke color - varies if any channel in the group is "stroke"
        let has_stroke_channel = channels.iter().any(|c| c.channel_type == "stroke");
        if has_stroke_channel {
            // Find the stroke channel and map its values
            if let Some(stroke_channel) = channels.iter().find(|c| c.channel_type == "stroke") {
                let colors = stroke_channel
                    .scale
                    .scale_scalars_to_colors(&domain_values)?;
                legend_config.stroke = ScalarOrArray::new_array(
                    colors.into_iter().map(ColorOrGradient::Color).collect(),
                );
            }
        } else {
            // No stroke channel in group - try to get constant stroke color from mark
            if let Some(color) = helpers::get_constant_color(
                "stroke",
                &primary_channel.related_channels,
                &self.mark_encodings,
            ) {
                legend_config.stroke = ScalarOrArray::new_scalar(color);
            } else {
                // No constant stroke color - use gray #666666 for non-stroke legends
                let color = ColorOrGradient::Color([0.4, 0.4, 0.4, 1.0]); // #666666 gray
                legend_config.stroke = ScalarOrArray::new_scalar(color);
            }
        }

        // Set stroke width - varies if any channel in the group is "stroke_width"
        let has_width_channel = channels.iter().any(|c| c.channel_type == "stroke_width");
        if has_width_channel {
            // Find the stroke_width channel and map its values
            if let Some(width_channel) = channels.iter().find(|c| c.channel_type == "stroke_width")
            {
                let widths = width_channel
                    .scale
                    .scale_scalars_to_numeric(&domain_values)?;
                legend_config.stroke_width = ScalarOrArray::new_array(widths);
            }
        } else {
            // Try to get constant stroke width from mark
            let width = helpers::get_constant_f32(
                "stroke_width",
                &primary_channel.related_channels,
                &self.mark_encodings,
            )
            .unwrap_or(default_stroke_width);
            legend_config.stroke_width = ScalarOrArray::new_scalar(width);
        }

        // Set stroke dash - varies if any channel in the group is "stroke_dash"
        let has_dash_channel = channels.iter().any(|c| c.channel_type == "stroke_dash");
        if has_dash_channel {
            // Find the stroke_dash channel and process it
            if let Some(dash_channel) = channels.iter().find(|c| c.channel_type == "stroke_dash") {
                // Vary dash pattern based on the dash channel's scale
                let mut dash_patterns = dash_channel
                    .scale
                    .scale_scalars_to_dash_patterns(&domain_values);

                // Replace empty patterns (solid lines) with a dash pattern for uniform processing
                // Use a slightly shorter dash to account for visual alignment with dashed patterns
                for p in dash_patterns.iter_mut().flatten() {
                    if p.is_empty() {
                        *p = vec![30.0, 0.0];
                    }
                }

                tracing::debug!(
                    channel = channel_name.as_str(),
                    domain_values = ?domain_values,
                    dash_patterns = ?dash_patterns,
                    "Line legend dash patterns"
                );

                // Use 32 as the target legend length - all patterns are designed to align at this length
                let max_legend_length = 32.0;

                // Now calculate optimal length for each pattern
                let mut individual_lengths = Vec::new();

                // Calculate cap extension first as we'll need it for determining actual_max_length
                let cap_extension = if self.stroke_cap == StrokeCap::Round {
                    default_stroke_width // Add stroke width for rounded caps at both ends
                } else {
                    0.0
                };

                let mut actual_max_length_without_caps: f32 = 0.0; // Track the longest dash pattern

                // First pass: calculate lengths for all dash patterns
                for pattern in dash_patterns.iter() {
                    if let Some(pattern) = pattern.as_ref() {
                        // Calculate how many complete dash segments fit within max_legend_length
                        let mut current_pos = 0.0;
                        let mut last_valid_length = 0.0;
                        let mut is_dash = true; // Start with a dash segment
                        let mut pattern_idx = 0;

                        // Simulate drawing the pattern
                        // We want to fit complete dash-gap pairs where possible
                        while current_pos <= max_legend_length {
                            let segment_length = pattern[pattern_idx];
                            let next_pos = current_pos + segment_length;

                            if is_dash {
                                // This is a dash segment
                                if next_pos <= max_legend_length {
                                    // Dash fits completely
                                    last_valid_length = next_pos;
                                } else {
                                    // Dash would exceed limit, don't include it
                                    break;
                                }
                            } else {
                                // This is a gap - we include it if the next dash will also fit
                                // Look ahead to see if there's room for the next dash
                                let next_pattern_idx = (pattern_idx + 1) % pattern.len();
                                let next_dash_length = pattern[next_pattern_idx];
                                if next_pos + next_dash_length > max_legend_length {
                                    // Next dash won't fit, so stop here
                                    break;
                                }
                            }

                            current_pos = next_pos;
                            is_dash = !is_dash;
                            pattern_idx = (pattern_idx + 1) % pattern.len();
                        }

                        // Make sure we show at least some pattern
                        if last_valid_length == 0.0 {
                            last_valid_length = pattern[0].min(max_legend_length); // At least show first dash
                        }

                        actual_max_length_without_caps =
                            actual_max_length_without_caps.max(last_valid_length);
                    }
                }

                // If no dash patterns were found, use the theoretical max
                if actual_max_length_without_caps == 0.0 {
                    actual_max_length_without_caps = max_legend_length;
                }

                tracing::debug!(
                    actual_max = actual_max_length_without_caps,
                    theoretical_max = max_legend_length,
                    cap_extension = cap_extension,
                    "Calculated actual max length from dash patterns"
                );

                // Second pass: assign lengths
                for (i, pattern) in dash_patterns.iter().enumerate() {
                    let optimal_length = if let Some(pattern) = pattern.as_ref() {
                        // Calculate how many complete dash segments fit within max_legend_length
                        // All patterns including [32, 0] for solid lines are processed uniformly
                        let mut current_pos = 0.0;
                        let mut last_valid_length = 0.0;
                        let mut is_dash = true; // Start with a dash segment
                        let mut pattern_idx = 0;

                        // Simulate drawing the pattern
                        // We want to fit complete dash-gap pairs where possible
                        while current_pos <= max_legend_length {
                            let segment_length = pattern[pattern_idx];
                            let next_pos = current_pos + segment_length;

                            if is_dash {
                                // This is a dash segment
                                if next_pos <= max_legend_length {
                                    // Dash fits completely
                                    last_valid_length = next_pos;
                                } else {
                                    // Dash would exceed limit, don't include it
                                    break;
                                }
                            } else {
                                // This is a gap - we include it if the next dash will also fit
                                // Look ahead to see if there's room for the next dash
                                let next_pattern_idx = (pattern_idx + 1) % pattern.len();
                                let next_dash_length = pattern[next_pattern_idx];
                                if next_pos + next_dash_length > max_legend_length {
                                    // Next dash won't fit, so stop here
                                    break;
                                }
                            }

                            current_pos = next_pos;
                            is_dash = !is_dash;
                            pattern_idx = (pattern_idx + 1) % pattern.len();
                        }

                        // Make sure we show at least some pattern
                        if last_valid_length == 0.0 && !pattern.is_empty() {
                            last_valid_length = pattern[0].min(max_legend_length); // At least show first dash
                        }

                        last_valid_length
                    } else {
                        // No pattern (solid line)
                        actual_max_length_without_caps
                    };

                    individual_lengths.push(optimal_length);

                    tracing::trace!(
                        index = i,
                        pattern = ?pattern,
                        length = optimal_length,
                        max_length = max_legend_length,
                        "Dash pattern"
                    );
                }

                tracing::trace!(max_legend_length = max_legend_length, "Max legend length");

                // Set individual lengths for each pattern (cap_extension already calculated above)
                legend_config.line_length = ScalarOrArray::new_array(
                    individual_lengths
                        .into_iter()
                        .map(|l| l + cap_extension)
                        .collect(),
                );
                legend_config.stroke_dash = ScalarOrArray::new_array(dash_patterns);
            }
        } else {
            // Try to get constant stroke dash from mark
            if let Some(pattern_str) = helpers::get_constant_string(
                "stroke_dash",
                &primary_channel.related_channels,
                &self.mark_encodings,
            ) {
                let dash = Self::convert_dash_pattern(&pattern_str);
                legend_config.stroke_dash = ScalarOrArray::new_scalar(dash);
            } else {
                // Use default (solid)
                legend_config.stroke_dash = ScalarOrArray::new_scalar(None);
            }
        }


        tracing::debug!(
            channel = channel_name.as_str(),
            stroke = ?legend_config.stroke.as_vec(8, None),
            stroke_width = ?legend_config.stroke_width.as_vec(8, None),
            stroke_dash = ?legend_config.stroke_dash.as_vec(8, None),
            line_length = ?legend_config.line_length.as_vec(8, None),
            "Line legend config"
        );

        let mut legend_group = make_line_legend(&legend_config)?;

        // Update position and add debug stroke
        legend_group.origin = [x, y];
        legend_group.zindex = Some(10);

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
        _ => format!("{:?}", value),
    }
}
