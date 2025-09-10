//! Rectangle/bar legend renderer for discrete scales

use crate::error::AvengerChartError;
use crate::legend::Legend;
use crate::legend_renderer::{LegendChannel, LegendRenderer, helpers};
use crate::marks::channel::ChannelValue;
use crate::scales::{ConfiguredScaleLegendExt, DomainValues};
use crate::utils::ScalarValueHelpers;
use avenger_common::types::{ColorOrGradient, SymbolShape};
use avenger_common::value::ScalarOrArray;
use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend};
use avenger_scenegraph::marks::group::SceneGroup;
use std::collections::HashMap;

/// Rectangle legend renderer for rect/bar marks
#[derive(Default)]
pub struct RectLegendRenderer {
    /// Map of mark encodings from the plot
    mark_encodings: HashMap<String, ChannelValue>,
}

impl RectLegendRenderer {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LegendRenderer for RectLegendRenderer {
    fn name(&self) -> &'static str {
        "RectLegendRenderer"
    }

    fn can_render(&self, channels: &[LegendChannel]) -> bool {
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

        // Default values for rect marks - always use square shape
        let (
            default_size,
            _default_shape,
            default_angle,
            default_fill,
            default_stroke,
            default_stroke_width,
        ) = (
            64.0,
            "square".to_string(),
            0.0,
            "#4682b4".to_string(),
            "#000000".to_string(),
            1.0,
        );

        // Get size value - use constant from mark if available
        let size_value = helpers::get_constant_f32(
            "size",
            &primary_channel.related_channels,
            &self.mark_encodings,
        )
        .unwrap_or(default_size as f32);

        // Create legend configuration
        let mut legend_config = SymbolLegendConfig {
            title: config.title.clone(),
            text: ScalarOrArray::new_scalar("".to_string()), // Will be set later
            shape: ScalarOrArray::new_scalar(
                SymbolShape::from_vega_str("square").unwrap_or_default(),
            ), // Always use square for rect marks
            size: ScalarOrArray::new_scalar(size_value),
            angle: ScalarOrArray::new_scalar(default_angle as f32),
            fill: ScalarOrArray::new_scalar(crate::utils::parse_color_string_strict(
                &default_fill,
            )?),
            stroke: ScalarOrArray::new_scalar(crate::utils::parse_color_string_strict(
                &default_stroke,
            )?),
            stroke_width: Some(default_stroke_width as f32),
            inner_width: 0.0,
            inner_height: 100.0,
            outer_margin: 0.0,
            text_padding: 2.0,
            background_fill: match config.background_fill.as_ref() {
                Some(f) => Some(crate::utils::parse_color_string_strict(f)?),
                None => None,
            },
            background_stroke: match config.background_stroke.as_ref() {
                Some(s) => Some(crate::utils::parse_color_string_strict(s)?),
                None => None,
            },
            background_corner_radius: config.background_corner_radius,
            background_padding: config.background_padding,
            title_color: match config.title_color.as_ref() {
                Some(c) => {
                    let color = crate::utils::parse_color_string_strict(c)?;
                    match color {
                        ColorOrGradient::Color(color) => Some(color),
                        _ => {
                            return Err(AvengerChartError::InternalError(format!(
                                "Legend title color '{}' parsed to gradient, expected solid color",
                                c
                            )));
                        }
                    }
                }
                None => None,
            },
            label_color: match config.label_color.as_ref() {
                Some(c) => {
                    let color = crate::utils::parse_color_string_strict(c)?;
                    match color {
                        ColorOrGradient::Color(color) => Some(color),
                        _ => {
                            return Err(AvengerChartError::InternalError(format!(
                                "Legend label color '{}' parsed to gradient, expected solid color",
                                c
                            )));
                        }
                    }
                }
                None => None,
            },
            title_font_family: None,
            title_font_size: None,
            title_font_weight: None,
            label_font_family: None,
            label_font_size: None,
            label_font_weight: None,
        };

        // Set typography from legend config
        if let Some(ref family) = config.title_font_family {
            legend_config.title_font_family = Some(family.clone());
        }
        if let Some(size) = config.title_font_size {
            legend_config.title_font_size = Some(size);
        }
        if let Some(weight) = config.title_font_weight {
            legend_config.title_font_weight = Some(avenger_text::types::FontWeight::Number(weight));
        }
        if let Some(ref family) = config.label_font_family {
            legend_config.label_font_family = Some(family.clone());
        }
        if let Some(size) = config.label_font_size {
            legend_config.label_font_size = Some(size);
        }
        if let Some(weight) = config.label_font_weight {
            legend_config.label_font_weight = Some(avenger_text::types::FontWeight::Number(weight));
        }

        // Use constant values from mark if available (and not the legend channel itself)
        if channel_name != "fill" && channel_name != "color" {
            if let Some(color) = helpers::get_constant_color(
                "fill",
                &primary_channel.related_channels,
                &self.mark_encodings,
            ) {
                legend_config.fill = ScalarOrArray::new_scalar(color);
            }
        }

        if channel_name != "stroke" {
            if let Some(color) = helpers::get_constant_color(
                "stroke",
                &primary_channel.related_channels,
                &self.mark_encodings,
            ) {
                legend_config.stroke = ScalarOrArray::new_scalar(color);
            }
        }

        if channel_name != "stroke_width" {
            if let Some(width) = helpers::get_constant_f32(
                "stroke_width",
                &primary_channel.related_channels,
                &self.mark_encodings,
            ) {
                legend_config.stroke_width = Some(width);
            }
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
