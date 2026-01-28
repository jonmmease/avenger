//! Zero-dimensional coordinate system
//!
//! The ZeroDCoord type represents a zero-dimensional coordinate system - essentially
//! a single point with no spatial extent.

use crate::coords::{CoordinateSystem, CoordinateSystemTransform, PointGeometry};
use crate::error::AvengerChartError;
use crate::guide::NoGuide;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A zero-dimensional coordinate system
///
/// Represents a 0D space (a single point) where marks have no spatial extent
/// or position channels. Useful for legends and other non-spatial mark rendering.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ZeroDCoord;

impl ZeroDCoord {
    /// Create a new zero-dimensional coordinate system
    pub fn new() -> Self {
        Self
    }
}

impl CoordinateSystem for ZeroDCoord {
    type Guide = NoGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        // ZeroDCoord has no position channels (0D space)
        &[]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for ZeroDCoord {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, avenger_common::value::ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        let _ = position_values;
        use avenger_common::value::ScalarOrArray;

        // In 0D space, all points collapse to the center of the plot area
        let center_x = plot_width / 2.0;
        let center_y = plot_height / 2.0;

        // Determine the size of the output based on any channel data
        // (all channels should have the same length if they're arrays)
        let len = position_channels
            .values()
            .find_map(|v| match v.value() {
                avenger_common::value::ScalarOrArrayValue::Array(arr) => Some(arr.len()),
                _ => None,
            })
            .unwrap_or(1);

        // Return center point(s) - either scalar or array of same center point
        let (x, y) = if len == 1 {
            (
                ScalarOrArray::new_scalar(center_x),
                ScalarOrArray::new_scalar(center_y),
            )
        } else {
            (
                ScalarOrArray::new_array(vec![center_x; len]),
                ScalarOrArray::new_array(vec![center_y; len]),
            )
        };

        Ok(Box::new(PointGeometry { x, y }))
    }

    fn default_range(
        &self,
        _channel: &str,
        _plot_area_width: f64,
        _plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        // No ranges in 0D space
        None
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::scalar::ScalarValue> {
        // Zero-dimensional coordinate system has no positional channels
        HashMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_zerod_transform_to_center() {
        // Create ZeroD coordinate system
        let coord = ZeroDCoord::new();
        let coord_transform = coord.create_transform();

        // Test with empty position channels (single point)
        let position_channels = HashMap::new();
        let geometry = coord_transform
            .transform(&position_channels, None, 100.0, 100.0)
            .unwrap();
        let point_geometry = geometry
            .as_any()
            .downcast_ref::<PointGeometry>()
            .expect("Expected PointGeometry");

        // Verify single point is at center (50, 50)
        use avenger_common::value::ScalarOrArrayValue;
        match (point_geometry.x.value(), point_geometry.y.value()) {
            (ScalarOrArrayValue::Scalar(x_val), ScalarOrArrayValue::Scalar(y_val)) => {
                assert_eq!(*x_val, 50.0);
                assert_eq!(*y_val, 50.0);
            }
            _ => panic!("Expected scalar values for single point"),
        }

        // Test with array data (should return arrays of center points)
        let mut position_channels_with_data = HashMap::new();
        position_channels_with_data.insert(
            "dummy",
            avenger_common::value::ScalarOrArray::new_array(vec![1.0, 2.0, 3.0]),
        );

        let geometry_arr = coord_transform
            .transform(&position_channels_with_data, None, 100.0, 100.0)
            .unwrap();
        let point_geometry_arr = geometry_arr
            .as_any()
            .downcast_ref::<PointGeometry>()
            .expect("Expected PointGeometry");

        match (point_geometry_arr.x.value(), point_geometry_arr.y.value()) {
            (ScalarOrArrayValue::Array(x_vals), ScalarOrArrayValue::Array(y_vals)) => {
                assert_eq!(x_vals.len(), 3);
                assert_eq!(y_vals.len(), 3);
                for i in 0..3 {
                    assert_eq!(x_vals[i], 50.0);
                    assert_eq!(y_vals[i], 50.0);
                }
            }
            _ => panic!("Expected array values for multiple points"),
        }
    }
}
