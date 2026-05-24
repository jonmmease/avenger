use std::collections::HashMap;

use datafusion::scalar::ScalarValue;
use serde::{Deserialize, Serialize};

use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};

use crate::{
    coords::{
        CoordinateSystem, CoordinateSystemTransform, CoordinateSystemTransformCore, PointGeometry,
    },
    error::AvengerChartError,
    scales::{PlotAreaRangeEndpoint, ScaleRangeBinding},
};

use super::PolarGuide;

/// Polar coordinate system with radial and angular axes
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Polar {}

impl Polar {
    pub fn new() -> Self {
        Self {}
    }
}

impl CoordinateSystem for Polar {
    type Guide = PolarGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &["r", "theta"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for Polar {
    fn required_channels(&self) -> &'static [&'static str] {
        &["r", "theta"]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        let _ = position_values;

        // Get r and theta channels
        let r = position_channels.get("r").ok_or_else(|| {
            AvengerChartError::InternalError("Missing r position channel".to_string())
        })?;

        let theta = position_channels.get("theta").ok_or_else(|| {
            AvengerChartError::InternalError("Missing theta position channel".to_string())
        })?;

        // Calculate center point
        let center_x = plot_width / 2.0;
        let center_y = plot_height / 2.0;

        // Convert polar to Cartesian coordinates
        // x = center_x + r * cos(theta)
        // y = center_y + r * sin(theta)
        let (x, y) = match (r.value(), theta.value()) {
            (ScalarOrArrayValue::Scalar(r_val), ScalarOrArrayValue::Scalar(theta_val)) => {
                let x = center_x + r_val * theta_val.cos();
                let y = center_y + r_val * theta_val.sin();
                (ScalarOrArray::new_scalar(x), ScalarOrArray::new_scalar(y))
            }
            (ScalarOrArrayValue::Array(r_vals), ScalarOrArrayValue::Array(theta_vals)) => {
                if r_vals.len() != theta_vals.len() {
                    return Err(AvengerChartError::InternalError(
                        "r and theta arrays must have the same length".to_string(),
                    ));
                }

                let x_vals: Vec<f32> = r_vals
                    .iter()
                    .zip(theta_vals.iter())
                    .map(|(r, theta)| center_x + r * theta.cos())
                    .collect();

                let y_vals: Vec<f32> = r_vals
                    .iter()
                    .zip(theta_vals.iter())
                    .map(|(r, theta)| center_y + r * theta.sin())
                    .collect();

                (
                    ScalarOrArray::new_array(x_vals),
                    ScalarOrArray::new_array(y_vals),
                )
            }
            _ => {
                return Err(AvengerChartError::InternalError(
                    "r and theta must both be scalars or both be arrays".to_string(),
                ));
            }
        };

        Ok(Box::new(PointGeometry { x, y }))
    }

    fn default_range_binding(&self, channel: &str) -> Option<ScaleRangeBinding> {
        match channel {
            "theta" => Some(ScaleRangeBinding::fixed_interval(
                0.0,
                2.0 * std::f64::consts::PI,
            )),
            "r" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::ZERO,
                PlotAreaRangeEndpoint::HALF_MIN_DIMENSION,
            )),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        let mut options = HashMap::new();

        // Get domain and range kinds directly from scale implementation
        let domain_kind = scale_impl.domain_kind();
        let range_kind = scale_impl.range_kind();

        // Apply polar-specific defaults
        if channel == "r" {
            // Radial axis typically starts at zero unless explicitly overridden
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                options.insert("zero".to_string(), ScalarValue::Boolean(Some(true)));
                options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));
                options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
            }
        } else if channel == "theta" {
            // Angular values are typically in radians or degrees
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                // Nice domain for better tick values
                options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));
            }
        }

        options
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for Polar {
    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}
