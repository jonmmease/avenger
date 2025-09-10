//! Zero-dimensional coordinate system
//!
//! The ZeroDCoord type represents a zero-dimensional coordinate system - essentially
//! a single point with no spatial extent. This is useful in contexts where marks need
//! to be rendered without any coordinate mapping, such as:
//!
//! - Legend symbols that show mark appearance without position
//! - Default value extraction from marks
//! - Standalone mark previews
//!
//! # Conceptual Model
//!
//! In a 0D coordinate system, there are no position channels, no axes, and no spatial
//! transformations. Marks rendered in this system appear at a fixed location without
//! any data-driven positioning.
//!
//! # Important
//!
//! ZeroDCoord should NOT be used for actual data visualization. All spatial methods
//! will panic with `unreachable!()` as they are meaningless in zero dimensions.

use crate::axis::Axis;
use crate::coords::{CoordinateSystem, OverflowSpaceRequirement};
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use std::any::Any;
use std::collections::HashMap;

/// A zero-dimensional coordinate system
///
/// Represents a 0D space (a single point) where marks have no spatial extent
/// or position channels. Useful for legends and other non-spatial mark rendering.
#[derive(Debug, Default, Clone)]
pub struct ZeroDCoord;

impl ZeroDCoord {
    /// Create a new zero-dimensional coordinate system
    pub fn new() -> Self {
        Self
    }
}

/// A placeholder axis for the zero-dimensional coordinate system
#[derive(Debug, Clone)]
pub struct ZeroDAxis {
    // No axes exist in 0D space
}

impl Axis for ZeroDAxis {
    fn clone_box(&self) -> Box<dyn Axis> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

#[async_trait::async_trait]
impl CoordinateSystem for ZeroDCoord {
    type Axis = ZeroDAxis;

    fn required_channels(&self) -> &'static [&'static str] {
        // ZeroDCoord has no position channels (0D space)
        &[]
    }

    fn default_range(&self, _channel: &str, _width: f64, _height: f64) -> Option<(f64, f64)> {
        // No ranges in 0D space
        None
    }

    fn create_default_axes(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis>
    where
        Self: Sized,
    {
        // No axes exist in zero-dimensional space
        HashMap::new()
    }

    async fn measure_guide_overflow(
        &self,
        _axes: HashMap<String, Self::Axis>,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _width_estimate: f32,
        _height_estimate: f32,
        _theme: &crate::theme::Theme,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        // No guides exist in 0D space, so no overflow
        Ok(OverflowSpaceRequirement {
            top: 0.0,
            bottom: 0.0,
            left: 0.0,
            right: 0.0,
        })
    }

    async fn render_axes(
        &self,
        _axes: &HashMap<String, Self::Axis>,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _padding: &crate::render::Padding,
        _theme: &crate::theme::Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // No axes exist in 0D space
        Ok(Vec::new())
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Clip {
        // No clipping needed in 0D space
        Clip::None
    }

    fn transform_to_plot_coords(
        &self,
        position_channels: &HashMap<&str, avenger_common::value::ScalarOrArray<f32>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<
        (
            avenger_common::value::ScalarOrArray<f32>,
            avenger_common::value::ScalarOrArray<f32>,
        ),
        AvengerChartError,
    > {
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
        if len == 1 {
            Ok((
                ScalarOrArray::new_scalar(center_x),
                ScalarOrArray::new_scalar(center_y),
            ))
        } else {
            Ok((
                ScalarOrArray::new_array(vec![center_x; len]),
                ScalarOrArray::new_array(vec![center_y; len]),
            ))
        }
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

        // Test with empty position channels (single point)
        let position_channels = HashMap::new();
        let (x, y) = coord
            .transform_to_plot_coords(&position_channels, 100.0, 100.0)
            .unwrap();

        // Verify single point is at center (50, 50)
        use avenger_common::value::ScalarOrArrayValue;
        match (x.value(), y.value()) {
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

        let (x_arr, y_arr) = coord
            .transform_to_plot_coords(&position_channels_with_data, 100.0, 100.0)
            .unwrap();

        match (x_arr.value(), y_arr.value()) {
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
