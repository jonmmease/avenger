use std::{any::Any, collections::HashMap};

use avenger_chart_core::{
    AvengerChartError, CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
    CoordinateSystemTransformCore, InteractionPointInversionRequest, PlotAreaRangeEndpoint,
    PlotGeometry, PointGeometry, ScaleRangeBinding,
};
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::marks::subplot::{CARTESIAN_SUBPLOT_X_CHANNEL, CARTESIAN_SUBPLOT_Y_CHANNEL};

fn cartesian_range_channel(channel: &str) -> &str {
    match channel {
        CARTESIAN_SUBPLOT_X_CHANNEL => "x",
        CARTESIAN_SUBPLOT_Y_CHANNEL => "y",
        _ => channel,
    }
}

/// Cartesian coordinate system with x and y axes.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Cartesian;

impl CoordinateSystemCore for Cartesian {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }
}

impl CoordinateSystem for Cartesian {
    type Guide = crate::guide::CartesianGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for Cartesian {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        let _ = position_values;
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

    fn default_range_binding(&self, channel: &str) -> Option<ScaleRangeBinding> {
        match cartesian_range_channel(channel) {
            "x" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::ZERO,
                PlotAreaRangeEndpoint::WIDTH,
            )),
            "y" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::HEIGHT,
                PlotAreaRangeEndpoint::ZERO,
            )),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        let domain_kind = scale_impl.domain_kind();
        let range_kind = scale_impl.range_kind();
        let scale_type = scale_impl.scale_type();

        let mut options = HashMap::new();

        let channel = cartesian_range_channel(channel);
        if channel == "x" || channel == "y" {
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                if channel == "y" && scale_type == "linear" {
                    options.insert("zero".to_string(), ScalarValue::Boolean(Some(true)));
                }

                options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));
                options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
            } else if domain_kind == DomainKind::Categorical && range_kind == RangeKind::Continuous
            {
                let option_defs = scale_impl.option_definitions();
                if option_defs.iter().any(|def| def.name == "padding") && scale_type == "band" {
                    options.insert("padding".to_string(), ScalarValue::Float64(Some(0.1)));
                }
            }
        }

        options
    }

    fn interaction_invertible_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn invert_interaction_point(
        &self,
        request: InteractionPointInversionRequest<'_>,
    ) -> Result<IndexMap<String, ScalarValue>, AvengerChartError> {
        let mut inverted = IndexMap::new();
        for channel in request.channels {
            let range_value = match *channel {
                "x" => request.local_point[0],
                "y" => request.local_point[1],
                other => {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Cartesian coordinate inversion does not support channel '{other}'"
                    )));
                }
            };
            let scale = request.scales.get(*channel).ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "Missing configured scale for coordinate channel '{channel}'"
                ))
            })?;
            // The default Cartesian y range binds to [plot_area_height, 0], so
            // invert_scalar already handles the reversed range; no special-case
            // arithmetic is needed here.
            let value = scale.invert_scalar(range_value).map_err(|err| {
                AvengerChartError::InvalidArgument(format!(
                    "Cannot invert coordinate channel '{channel}' (scale type {}): {err}",
                    scale.scale_impl.scale_type()
                ))
            })?;
            inverted.insert(
                (*channel).to_string(),
                ScalarValue::Float64(Some(value as f64)),
            );
        }
        Ok(inverted)
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for Cartesian {
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
    use avenger_scales::scales::linear::LinearScale;

    #[test]
    fn cartesian_invertible_channels_are_x_and_y() {
        assert_eq!(Cartesian.interaction_invertible_channels(), &["x", "y"]);
    }

    #[test]
    fn invert_x_maps_local_point_through_scale() {
        let mut scales = HashMap::new();
        scales.insert(
            "x".to_string(),
            LinearScale::configured((0.0, 10.0), (0.0, 100.0)),
        );
        let inverted = Cartesian
            .invert_interaction_point(InteractionPointInversionRequest {
                local_point: [25.0, 0.0],
                plot_area_width: 100.0,
                plot_area_height: 100.0,
                channels: &["x"],
                scales: &scales,
            })
            .expect("invert x");
        match inverted.get("x") {
            Some(ScalarValue::Float64(Some(value))) => assert!((value - 2.5).abs() < 1e-6),
            other => panic!("expected x=2.5, got {other:?}"),
        }
    }

    #[test]
    fn invert_y_handles_reversed_range() {
        // Cartesian y range binds to [plot_area_height, 0], so a local y near the
        // top of the plot inverts to a high domain value.
        let mut scales = HashMap::new();
        scales.insert(
            "y".to_string(),
            LinearScale::configured((0.0, 10.0), (100.0, 0.0)),
        );
        let inverted = Cartesian
            .invert_interaction_point(InteractionPointInversionRequest {
                local_point: [0.0, 25.0],
                plot_area_width: 100.0,
                plot_area_height: 100.0,
                channels: &["y"],
                scales: &scales,
            })
            .expect("invert y");
        match inverted.get("y") {
            Some(ScalarValue::Float64(Some(value))) => assert!((value - 7.5).abs() < 1e-6),
            other => panic!("expected y=7.5, got {other:?}"),
        }
    }

    #[test]
    fn invert_missing_scale_is_invalid_argument() {
        let scales = HashMap::new();
        let err = Cartesian
            .invert_interaction_point(InteractionPointInversionRequest {
                local_point: [25.0, 0.0],
                plot_area_width: 100.0,
                plot_area_height: 100.0,
                channels: &["x"],
                scales: &scales,
            })
            .expect_err("missing scale should be invalid");
        assert!(matches!(err, AvengerChartError::InvalidArgument(_)));
    }
}
