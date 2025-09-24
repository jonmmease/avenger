use crate::cartesian::{CartesianAxis, CartesianGuide, axis::AxisPosition};
use crate::coords::{CoordinateSystem, PointGeometry, extract_channel_title_from_marks};
use crate::error::AvengerChartError;
use crate::marks::Mark;
use avenger_scenegraph::marks::group::Clip;
use std::collections::HashMap;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::guide::CoordinateGuideBuilder;

/// Cartesian coordinate system with x and y axes
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Cartesian;

#[async_trait::async_trait]
impl CoordinateSystem for Cartesian {
    type Guide = CartesianGuide;
    type PlotGeometry = PointGeometry;

    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<(f64, f64)> {
        match channel {
            "x" => Some((0.0, width)),
            "y" => Some((height, 0.0)), // Inverted for screen coords
            _ => None,
        }
    }

    fn create_default_axes(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        marks: &[Arc<dyn Mark<Self>>],
    ) -> HashMap<String, <Self::Guide as CoordinateGuideBuilder>::Axis> {
        let mut default_axes = HashMap::new();

        // Always create default axes for x and y channels if they have scales
        // User axes will be merged with these defaults later
        for channel in ["x", "y"] {
            if scales.get(channel).is_some() {
                // Extract title from mark encodings, fall back to channel name if not found
                let title = extract_channel_title_from_marks(marks, channel);

                // Determine if grid should be enabled based on scale type
                let grid = if let Some(scale) = scales.get(channel) {
                    scale.ticks(None).is_ok()
                } else {
                    false
                };

                // Determine axis position based on channel
                let position = match channel {
                    "x" => AxisPosition::Bottom,
                    "y" => AxisPosition::Left,
                    _ => unreachable!(),
                };

                let mut axis = CartesianAxis::new().visible(true).position(position);

                if let Some(title) = title {
                    axis = axis.title(title);
                }

                axis = axis.grid(grid);

                default_axes.insert(channel.to_string(), axis);
            }
        }

        default_axes
    }

    fn create_default_guide(
        &self,
        axes: HashMap<String, <Self::Guide as CoordinateGuideBuilder>::Axis>,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _marks: &[Arc<dyn Mark<Self>>],
    ) -> Self::Guide {
        let mut guide = CartesianGuide::new();
        guide.set_axes(axes);
        guide
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Clip {
        // Rectangular clipping for Cartesian coordinates
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, avenger_common::value::ScalarOrArray<f32>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<PointGeometry, AvengerChartError> {
        // In Cartesian coordinates, the scaled values are already in plot coordinates
        // Just extract x and y from the position channels
        let x = position_channels
            .get("x")
            .ok_or_else(|| {
                AvengerChartError::InternalError("Missing x position channel".to_string())
            })?
            .clone();

        let y = position_channels
            .get("y")
            .ok_or_else(|| {
                AvengerChartError::InternalError("Missing y position channel".to_string())
            })?
            .clone();

        Ok(PointGeometry { x, y })
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::logical_expr::Expr> {
        use avenger_scales::scales::{DomainKind, RangeKind};
        use datafusion::logical_expr::lit;

        let mut options = HashMap::new();

        // Check if this is a position channel
        let is_position = matches!(channel, "x" | "x2" | "y" | "y2");
        let is_y_axis = matches!(channel, "y" | "y2");

        if is_position {
            let domain_kind = scale_impl.domain_kind();
            let range_kind = scale_impl.range_kind();
            let scale_type = scale_impl.scale_type();

            // For continuous numeric scales
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                // Y-axis scales typically include zero, X-axis scales don't necessarily
                if is_y_axis && scale_type == "linear" {
                    options.insert("zero".to_string(), lit(true));
                }

                // Nice domain for better tick values
                options.insert("nice".to_string(), lit(true));

                // Pixel-aligned positions for crisp rendering
                options.insert("round".to_string(), lit(true));
            }
            // For temporal scales
            else if domain_kind == DomainKind::Temporal && range_kind == RangeKind::Continuous {
                // Pixel-aligned positions
                options.insert("round".to_string(), lit(true));
            }
            // For categorical scales
            else if domain_kind == DomainKind::Categorical && range_kind == RangeKind::Continuous
            {
                // Only band scales support padding, not point scales
                if scale_type == "band" {
                    options.insert("padding".to_string(), lit(0.1));
                }
            }
        }

        options
    }
}
