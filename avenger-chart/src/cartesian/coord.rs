use crate::cartesian::CartesianGuide;
use crate::coords::{CoordinateSystem, CoordinateSystemTransform, PointGeometry};
use crate::error::AvengerChartError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Cartesian coordinate system with x and y axes
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Cartesian;

impl CoordinateSystem for Cartesian {
    type Guide = CartesianGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for Cartesian {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, avenger_common::value::ScalarOrArray<f32>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
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

        Ok(Box::new(PointGeometry { x, y }))
    }

    fn default_range(
        &self,
        channel: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match channel {
            "x" => Some((0.0, plot_area_width)),
            "y" => Some((plot_area_height, 0.0)),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::scalar::ScalarValue> {
        use avenger_scales::scales::{DomainKind, RangeKind};
        use datafusion::scalar::ScalarValue;

        // Get domain and range kinds directly from scale implementation
        let domain_kind = scale_impl.domain_kind();
        let range_kind = scale_impl.range_kind();
        let scale_type = scale_impl.scale_type();

        let mut options = HashMap::new();

        // Apply coordinate-specific defaults
        if channel == "x" || channel == "y" {
            // For quantitative scales (linear, log, etc.)
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                // Include zero for y-axis by default (bar charts) - but only for linear scales
                if channel == "y" && scale_type == "linear" {
                    options.insert("zero".to_string(), ScalarValue::Boolean(Some(true)));
                }

                // Nice domain for better tick values
                options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));

                // Pixel-aligned positions for crisp rendering
                options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
            }
            // For temporal scales
            else if domain_kind == DomainKind::Temporal && range_kind == RangeKind::Continuous {
                // Pixel-aligned positions
                options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
            }
            // For categorical scales
            else if domain_kind == DomainKind::Categorical && range_kind == RangeKind::Continuous
            {
                // Check if scale actually supports padding option
                // Band scales typically support padding, point scales may support different padding
                let option_defs = scale_impl.option_definitions();
                if option_defs.iter().any(|def| def.name == "padding") {
                    // Only set padding for band scales (which typically have padding default of 0.0)
                    // Point scales typically default to 0.5 padding already
                    if scale_type == "band" {
                        options.insert("padding".to_string(), ScalarValue::Float64(Some(0.1)));
                    }
                }
            }
        }

        options
    }
}
