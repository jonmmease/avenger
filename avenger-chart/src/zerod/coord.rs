//! Zero-dimensional coordinate system
//!
//! The ZeroDCoord type represents a zero-dimensional coordinate system - essentially
//! a single point with no spatial extent.

use crate::coords::{CoordinateSystem, CoordinateSystemTransform, PointGeometry};
use crate::error::AvengerChartError;
use crate::guide::{CoordinateGuideBuilder, NoGuide, OverflowSpaceRequirement};
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;
use std::sync::Arc;

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

#[async_trait::async_trait]
impl CoordinateSystem for ZeroDCoord {
    type Guide = NoGuide;
    type PlotGeometry = PointGeometry;

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
        _marks: &[Arc<dyn crate::marks::MarkRenderer>],
    ) -> HashMap<String, <Self::Guide as CoordinateGuideBuilder>::Axis> {
        // No axes exist in zero-dimensional space
        HashMap::new()
    }

    fn create_default_guide(
        &self,
        _axes: HashMap<String, <Self::Guide as CoordinateGuideBuilder>::Axis>,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _marks: &[Arc<dyn crate::marks::MarkRenderer>],
    ) -> Self::Guide {
        NoGuide::default()
    }

    async fn measure_guide_overflow(
        &self,
        _guide: &Self::Guide,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _width_estimate: f32,
        _height_estimate: f32,
        _theme: &dyn crate::theme::Theme,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        // No guides exist in 0D space, so no overflow
        Ok(OverflowSpaceRequirement {
            top: 0.0,
            bottom: 0.0,
            left: 0.0,
            right: 0.0,
        })
    }

    async fn render_guide(
        &self,
        _guide: &Self::Guide,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &crate::layout::LayoutBounds,
        _theme: &dyn crate::theme::Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // No guides exist in 0D space
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

    fn transform(
        &self,
        position_channels: &HashMap<&str, avenger_common::value::ScalarOrArray<f32>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<PointGeometry, AvengerChartError> {
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

        Ok(PointGeometry { x, y })
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransform for ZeroDCoord {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, avenger_common::value::ScalarOrArray<f32>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        let geom = <Self as CoordinateSystem>::transform(
            self,
            position_channels,
            plot_width,
            plot_height,
        )?;
        Ok(Box::new(geom))
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
        let geometry =
            <ZeroDCoord as CoordinateSystem>::transform(&coord, &position_channels, 100.0, 100.0)
                .unwrap();

        // Verify single point is at center (50, 50)
        use avenger_common::value::ScalarOrArrayValue;
        match (geometry.x.value(), geometry.y.value()) {
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

        let geometry_arr = <ZeroDCoord as CoordinateSystem>::transform(
            &coord,
            &position_channels_with_data,
            100.0,
            100.0,
        )
        .unwrap();

        match (geometry_arr.x.value(), geometry_arr.y.value()) {
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
