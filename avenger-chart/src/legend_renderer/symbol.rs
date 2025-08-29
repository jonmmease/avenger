//! Symbol legend renderer for discrete channels

use crate::error::AvengerChartError;
use crate::legend::Legend;
use crate::legend_renderer::{LegendChannel, LegendRenderer, helpers};
use crate::scales::{ConfiguredScaleLegendExt, DomainValues};
use avenger_common::types::{ColorOrGradient, SymbolShape};
use avenger_common::value::ScalarOrArray;
use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend};
use avenger_scenegraph::marks::group::SceneGroup;
use datafusion_common::ScalarValue;
use std::collections::HashMap;
use std::sync::Arc;

/// Symbol legend renderer for discrete channels
#[derive(Default)]
pub struct SymbolLegendRenderer {
    /// Plot reference for accessing mark data
    #[allow(dead_code)]
    plot_marks: Vec<Arc<dyn std::any::Any + Send + Sync>>,
    /// Map of mark encodings from the plot  
    mark_encodings: HashMap<String, crate::marks::channel::ChannelValue>,
    /// Whether the plot has rect marks
    has_rect_mark: bool,
}

impl SymbolLegendRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_rect_mark(&mut self, is_rect: bool) {
        self.has_rect_mark = is_rect;
    }

    pub fn with_plot_context<C: crate::coords::CoordinateSystem>(
        plot: &crate::plot::Plot<C>,
    ) -> Self {
        let mut mark_encodings = HashMap::new();
        let mut has_rect_mark = false;

        // Analyze mark encodings
        for mark in &plot.marks {
            let mark_type = mark.mark_type();
            if mark_type == "rect" {
                has_rect_mark = true;
            }

            // Collect encodings from symbol or rect marks
            if mark_type == "symbol" || mark_type == "rect" {
                let channels = mark.data_context().channels();
                for (channel, value) in channels {
                    mark_encodings.insert(channel.clone(), value.clone());
                }
            }
        }

        Self {
            plot_marks: Vec::new(), // We don't actually store the marks for now
            mark_encodings,
            has_rect_mark,
        }
    }
}

#[async_trait::async_trait]
impl LegendRenderer for SymbolLegendRenderer {
    fn can_render(&self, channels: &[LegendChannel]) -> bool {
        // Symbol legend can render discrete channels
        channels.iter().all(|c| {
            matches!(
                c.scale.scale_impl.scale_type(),
                "ordinal" | "band" | "point" | "threshold" | "quantile" | "quantize"
            )
        })
    }

    async fn render(
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

        // Create text labels - use special labels for threshold scales
        let text_values: Vec<String> =
            if primary_channel.scale.scale_impl.scale_type() == "threshold" {
                let labels = primary_channel.scale.domain_labels()?;
                tracing::debug!(
                    channel = channel_name.as_str(),
                    labels = ?labels,
                    "Threshold scale legend labels"
                );
                labels
            } else {
                domain_values.iter().map(format_scalar_value).collect()
            };

        // Get mark defaults - use rect defaults if we have rect marks, otherwise symbol defaults
        let (
            default_size,
            default_shape,
            default_angle,
            default_fill,
            default_stroke,
            default_stroke_width,
        ) = if self.has_rect_mark {
            // For rect marks, use fixed square shape and appropriate size
            (
                64.0,
                "square".to_string(),
                0.0,
                "#4682b4".to_string(),
                "#000000".to_string(),
                1.0,
            )
        } else {
            // Use symbol defaults
            (
                64.0,
                "circle".to_string(),
                0.0,
                "#4682b4".to_string(),
                "#000000".to_string(),
                1.0,
            )
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
            title: config.title.clone(),
            text: ScalarOrArray::new_array(text_values),
            inner_width: 0.0, // Don't offset internally, we'll position the whole group
            inner_height: 100.0, // Will be calculated by legend
            outer_margin: 0.0, // Don't offset legend entries
            text_padding: 2.0, // Consistent padding
            ..Default::default()
        };

        // Apply legend background styling if provided
        if let Some(pad) = config.background_padding {
            legend_config.background_padding = Some(pad);
            tracing::trace!(padding = pad, "Symbol legend padding set");
        } else {
            tracing::trace!("Symbol legend padding: None (will use default)");
        }
        if let Some(r) = config.background_corner_radius {
            legend_config.background_corner_radius = Some(r);
        }
        if let Some(ref fill_str) = config.background_fill {
            if let Some(color) = crate::utils::parse_color_string(fill_str) {
                legend_config.background_fill = Some(color);
            }
        }
        if let Some(ref stroke_str) = config.background_stroke {
            if let Some(color) = crate::utils::parse_color_string(stroke_str) {
                legend_config.background_stroke = Some(color);
            }
        }

        // Each legend only shows its own channel varying - no cross-channel variation

        // Shape channel
        let default_shape_parsed = parse_shape(&default_shape)?;
        legend_config.shape = ScalarOrArray::new_scalar(default_shape_parsed);

        if channel_name == "shape" {
            // Shape is the legend channel - map domain values to shapes
            let shape_names = {
                let names = primary_channel.scale.extract_shape_range();
                if !names.is_empty() {
                    names
                } else {
                    crate::scales::shape_defaults::DEFAULT_SHAPES
                        .iter()
                        .map(|&s| s.to_string())
                        .collect()
                }
            };

            let shapes: Result<Vec<_>, _> = domain_values
                .iter()
                .enumerate()
                .map(|(i, _)| parse_shape(&shape_names[i % shape_names.len()]))
                .collect();
            legend_config.shape = ScalarOrArray::new_array(shapes?);
        } else {
            // Shape channel exists but this legend is not for shape - use constant value if available
            if let Some(shape_str) = helpers::get_constant_string(
                "shape",
                &primary_channel.related_channels,
                &self.mark_encodings,
            )
            .await
            {
                legend_config.shape = ScalarOrArray::new_scalar(parse_shape(&shape_str)?);
            }
            // Otherwise keep the default shape
        }

        // Size channel
        legend_config.size = ScalarOrArray::new_scalar(default_size);
        if channel_name == "shape" {
            tracing::trace!(default_size = default_size, "Initial size set to default");
        }

        // Debug: log related channels
        tracing::debug!(
            channel = channel_name.as_str(),
            related_channels = ?primary_channel.related_channels.keys().collect::<Vec<_>>(),
            has_size = primary_channel.related_channels.contains_key("size"),
            "Checking for size channel"
        );

        if channel_name == "size" {
            // Size is the legend channel - map through scale
            let sizes = primary_channel.scale.map_values_numeric(&domain_values)?;
            legend_config.size = ScalarOrArray::new_array(sizes);
        } else {
            // Size channel exists but this legend is not for size - use constant value if available
            if let Some(size_value) = helpers::get_constant_f32(
                "size",
                &primary_channel.related_channels,
                &self.mark_encodings,
            )
            .await
            {
                tracing::debug!(
                    channel = channel_name.as_str(),
                    size_value = size_value,
                    "Setting legend size from constant value"
                );
                legend_config.size = ScalarOrArray::new_scalar(size_value);
            }
            // Otherwise keep the default size
        }

        if channel_name == "shape" {
            let sizes = legend_config.size.as_vec(3, None);
            tracing::trace!(sizes = ?sizes, "After size logic");
        }

        // Fill channel
        let default_fill_color = crate::utils::parse_color_string(&default_fill)
            .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]));
        legend_config.fill = ScalarOrArray::new_scalar(default_fill_color.clone());

        if channel_name == "fill" || channel_name == "color" {
            // Fill/color is the legend channel - map through scale
            let colors = if primary_channel.scale.scale_impl.scale_type() == "threshold" {
                // For threshold scales, get the range colors directly
                primary_channel.scale.range_colors()?
            } else {
                primary_channel.scale.map_values_colors(&domain_values)?
            };
            legend_config.fill =
                ScalarOrArray::new_array(colors.into_iter().map(ColorOrGradient::Color).collect());
        } else {
            // Fill channel exists but this legend is not for fill - use constant value if available
            if let Some(color) = helpers::get_constant_color(
                "fill",
                &primary_channel.related_channels,
                &self.mark_encodings,
            )
            .await
            {
                legend_config.fill = ScalarOrArray::new_scalar(color);
            }
            // Otherwise keep the default fill
        }

        // Stroke channel
        let default_stroke_color = crate::utils::parse_color_string(&default_stroke)
            .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]));
        legend_config.stroke = ScalarOrArray::new_scalar(default_stroke_color.clone());

        if channel_name == "stroke" {
            // Stroke is the legend channel - map through scale
            let colors = if primary_channel.scale.scale_impl.scale_type() == "threshold" {
                // For threshold scales, get the range colors directly
                primary_channel.scale.range_colors()?
            } else {
                primary_channel.scale.map_values_colors(&domain_values)?
            };
            legend_config.stroke =
                ScalarOrArray::new_array(colors.into_iter().map(ColorOrGradient::Color).collect());
        } else {
            // Stroke channel exists but this legend is not for stroke - use constant value if available
            if let Some(color) = helpers::get_constant_color(
                "stroke",
                &primary_channel.related_channels,
                &self.mark_encodings,
            )
            .await
            {
                legend_config.stroke = ScalarOrArray::new_scalar(color);
            }
            // Otherwise keep the default stroke
        }

        // Stroke width channel - start with default
        legend_config.stroke_width = Some(default_stroke_width);
        if channel_name == "shape" {
            tracing::trace!(stroke_width = default_stroke_width, "Stroke width set");
        }

        // Stroke width - use constant value if available
        if let Some(width) = helpers::get_constant_f32(
            "stroke_width",
            &primary_channel.related_channels,
            &self.mark_encodings,
        )
        .await
        {
            legend_config.stroke_width = Some(width);
        }

        // Angle channel
        legend_config.angle = ScalarOrArray::new_scalar(default_angle);

        if channel_name == "angle" {
            // Angle is the legend channel - map through scale
            let angles = primary_channel.scale.map_values_numeric(&domain_values)?;
            legend_config.angle = ScalarOrArray::new_array(angles);
        } else {
            // Angle channel exists but this legend is not for angle - use constant value if available
            if let Some(angle) = helpers::get_constant_f32(
                "angle",
                &primary_channel.related_channels,
                &self.mark_encodings,
            )
            .await
            {
                legend_config.angle = ScalarOrArray::new_scalar(angle);
            }
            // Otherwise keep the default angle
        }

        // Create the legend marks
        if channel_name == "shape" {
            let sizes = legend_config.size.as_vec(3, None);
            tracing::trace!(sizes = ?sizes, "Final config.size for shape");
        }
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
