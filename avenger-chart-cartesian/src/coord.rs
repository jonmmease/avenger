use std::{any::Any, collections::HashMap};

use avenger_chart_core::{
    AvengerChartError, CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
    CoordinateSystemTransformCore, PlotAreaRangeEndpoint, PlotGeometry, PointGeometry,
    ScaleRangeBinding,
};
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::common::ScalarValue;
use serde::{Deserialize, Serialize};

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
        match channel {
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

        if channel == "x" || channel == "y" {
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                if channel == "y" && scale_type == "linear" {
                    options.insert("zero".to_string(), ScalarValue::Boolean(Some(true)));
                }

                options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));
                options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
            } else if domain_kind == DomainKind::Temporal && range_kind == RangeKind::Continuous {
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
