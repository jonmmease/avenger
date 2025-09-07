//! Colorbar legend renderer for continuous color scales

use crate::error::AvengerChartError;
use crate::legend::Legend;
use crate::legend_renderer::{LegendChannel, LegendRenderer};
use crate::scales::{ConfiguredScaleLegendExt, DomainValues};
use avenger_guides::legend::colorbar::{ColorbarConfig, ColorbarOrientation};
use avenger_scenegraph::marks::group::SceneGroup;

/// Colorbar legend renderer for continuous color scales
#[derive(Default)]
pub struct ColorbarRenderer;

impl ColorbarRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl LegendRenderer for ColorbarRenderer {
    fn name(&self) -> &'static str {
        "ColorbarRenderer"
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

    fn render(
        &self,
        channels: &[LegendChannel],
        config: &Legend,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
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

        // Determine colorbar dimensions
        // When using Taffy layout, height is the allocated height
        // We should use this directly as the total colorbar height
        let colorbar_height = height;
        let colorbar_width = config.gradient_thickness.unwrap_or(15.0) as f32;

        let mut legend_config = ColorbarConfig {
            orientation: ColorbarOrientation::Right,
            dimensions: [width, height], // Available space for the colorbar
            colorbar_width: Some(colorbar_width),
            colorbar_height: Some(colorbar_height),
            colorbar_margin: Some(0.0), // No margin - align exactly with axis
            format_number: config.format_number.clone(),
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

        // Apply legend colors and typography from config
        if let Some(ref title_color) = config.title_color {
            if let Ok(color) = crate::utils::parse_color_string_strict(title_color) {
                if let avenger_common::types::ColorOrGradient::Color(c) = color {
                    legend_config.title_color = Some(c);
                }
            }
        }
        // Use tick_color for colorbar axis labels (not label_color which is for discrete legends)
        if let Some(ref tick_color) = config.tick_color {
            if let Ok(color) = crate::utils::parse_color_string_strict(tick_color) {
                if let avenger_common::types::ColorOrGradient::Color(c) = color {
                    legend_config.label_color = Some(c);
                }
            }
        }

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
        // Use tick typography for colorbar axis labels
        if let Some(ref family) = config.tick_font_family {
            legend_config.label_font_family = Some(family.clone());
        }
        if let Some(size) = config.tick_font_size {
            legend_config.label_font_size = Some(size);
        }
        if let Some(weight) = config.tick_font_weight {
            legend_config.label_font_weight = Some(avenger_text::types::FontWeight::Number(weight));
        }

        // Apply legend background styling if provided
        if let Some(pad) = config.background_padding {
            legend_config.background_padding = Some(pad);
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

        // Create the colorbar marks at origin [0, 0] (will be positioned by group origin)
        let plot_origin = [0.0, 0.0];
        let title = config.title.as_deref().unwrap_or("");

        let mut colorbar_group =
            make_colorbar_marks(configured_scale, title, plot_origin, &legend_config)?;

        // Position the colorbar group
        colorbar_group.origin = [x, y];

        // Set z-index
        colorbar_group.zindex = Some(10); // Legends above data but below title

        Ok(Some(colorbar_group))
    }
}
