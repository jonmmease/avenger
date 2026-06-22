use std::{any::Any, collections::HashMap};

use avenger_chart_core::{
    AvengerChartError, CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
    CoordinateSystemTransformCore, PlotAreaRangeEndpoint, PlotGeometry, PointGeometry,
    ScaleRangeBinding,
};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::common::ScalarValue;
use serde::{Deserialize, Serialize};

/// Polar coordinate system with radial and angular axes.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Polar {}

impl Polar {
    pub fn new() -> Self {
        Self {}
    }
}

impl CoordinateSystemCore for Polar {
    fn required_channels(&self) -> &'static [&'static str] {
        &["r", "theta"]
    }
}

impl CoordinateSystem for Polar {
    type Guide = crate::guide::PolarGuide;

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
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        let _ = position_values;
        let r = position_channels.get("r").ok_or_else(|| {
            AvengerChartError::InternalError("Missing r position channel".to_string())
        })?;

        let theta = position_channels.get("theta").ok_or_else(|| {
            AvengerChartError::InternalError("Missing theta position channel".to_string())
        })?;

        let center_x = plot_width / 2.0;
        let center_y = plot_height / 2.0;

        let (x, y) = match (r.value(), theta.value()) {
            (ScalarOrArrayValue::Scalar(r_val), ScalarOrArrayValue::Scalar(theta_val)) => {
                let (x, y) = polar_to_point(*r_val, *theta_val, center_x, center_y);
                (ScalarOrArray::new_scalar(x), ScalarOrArray::new_scalar(y))
            }
            (ScalarOrArrayValue::Array(r_vals), ScalarOrArrayValue::Array(theta_vals)) => {
                if r_vals.len() != theta_vals.len() {
                    return Err(AvengerChartError::InternalError(
                        "r and theta arrays must have the same length".to_string(),
                    ));
                }

                project_arrays(
                    r_vals.iter().copied(),
                    theta_vals.iter().copied(),
                    center_x,
                    center_y,
                )
            }
            (ScalarOrArrayValue::Scalar(r_val), ScalarOrArrayValue::Array(theta_vals)) => {
                project_arrays(
                    std::iter::repeat_n(*r_val, theta_vals.len()),
                    theta_vals.iter().copied(),
                    center_x,
                    center_y,
                )
            }
            (ScalarOrArrayValue::Array(r_vals), ScalarOrArrayValue::Scalar(theta_val)) => {
                project_arrays(
                    r_vals.iter().copied(),
                    std::iter::repeat_n(*theta_val, r_vals.len()),
                    center_x,
                    center_y,
                )
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

        let domain_kind = scale_impl.domain_kind();
        let range_kind = scale_impl.range_kind();

        if channel == "r" {
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                options.insert("zero".to_string(), ScalarValue::Boolean(Some(true)));
                options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));
                options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
            }
        } else if channel == "theta"
            && domain_kind == DomainKind::Numeric
            && range_kind == RangeKind::Continuous
        {
            options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));
        }

        options
    }
}

fn polar_to_point(r: f32, theta: f32, center_x: f32, center_y: f32) -> (f32, f32) {
    (center_x + r * theta.cos(), center_y + r * theta.sin())
}

fn project_arrays(
    r_vals: impl Iterator<Item = f32>,
    theta_vals: impl Iterator<Item = f32>,
    center_x: f32,
    center_y: f32,
) -> (ScalarOrArray<f32>, ScalarOrArray<f32>) {
    let (x_vals, y_vals): (Vec<_>, Vec<_>) = r_vals
        .zip(theta_vals)
        .map(|(r, theta)| polar_to_point(r, theta, center_x, center_y))
        .unzip();

    (
        ScalarOrArray::new_array(x_vals),
        ScalarOrArray::new_array(y_vals),
    )
}

#[typetag::serde]
impl CoordinateSystemTransform for Polar {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform(
        r: ScalarOrArray<f32>,
        theta: ScalarOrArray<f32>,
    ) -> Result<PointGeometry, AvengerChartError> {
        let mut channels = HashMap::new();
        channels.insert("r", r);
        channels.insert("theta", theta);
        let geometry = Polar::new().transform(&channels, None, 200.0, 100.0)?;
        geometry
            .as_any()
            .downcast_ref::<PointGeometry>()
            .cloned()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast test geometry".to_string(),
                )
            })
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn transforms_scalar_r_with_scalar_theta() {
        let geometry = transform(
            ScalarOrArray::new_scalar(10.0),
            ScalarOrArray::new_scalar(0.0),
        )
        .unwrap();

        let x = geometry.x.as_vec(1, None);
        let y = geometry.y.as_vec(1, None);
        assert_close(x[0], 110.0);
        assert_close(y[0], 50.0);
    }

    #[test]
    fn transforms_array_r_with_array_theta() {
        let geometry = transform(
            ScalarOrArray::new_array(vec![10.0, 20.0]),
            ScalarOrArray::new_array(vec![0.0, std::f32::consts::FRAC_PI_2]),
        )
        .unwrap();

        let x = geometry.x.as_vec(2, None);
        let y = geometry.y.as_vec(2, None);
        assert_close(x[0], 110.0);
        assert_close(y[0], 50.0);
        assert_close(x[1], 100.0);
        assert_close(y[1], 70.0);
    }

    #[test]
    fn transforms_scalar_r_with_array_theta() {
        let geometry = transform(
            ScalarOrArray::new_scalar(10.0),
            ScalarOrArray::new_array(vec![0.0, std::f32::consts::FRAC_PI_2]),
        )
        .unwrap();

        let x = geometry.x.as_vec(2, None);
        let y = geometry.y.as_vec(2, None);
        assert_close(x[0], 110.0);
        assert_close(y[0], 50.0);
        assert_close(x[1], 100.0);
        assert_close(y[1], 60.0);
    }

    #[test]
    fn transforms_array_r_with_scalar_theta() {
        let geometry = transform(
            ScalarOrArray::new_array(vec![10.0, 20.0]),
            ScalarOrArray::new_scalar(std::f32::consts::PI),
        )
        .unwrap();

        let x = geometry.x.as_vec(2, None);
        let y = geometry.y.as_vec(2, None);
        assert_close(x[0], 90.0);
        assert_close(y[0], 50.0);
        assert_close(x[1], 80.0);
        assert_close(y[1], 50.0);
    }

    #[test]
    fn rejects_mismatched_arrays() {
        let err = transform(
            ScalarOrArray::new_array(vec![10.0]),
            ScalarOrArray::new_array(vec![0.0, 1.0]),
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("r and theta arrays must have the same length")
        );
    }
}
