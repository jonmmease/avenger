use std::collections::HashMap;

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ScaleImpl;
use datafusion::common::ScalarValue;

use crate::{AvengerChartError, PlotGeometry, ScaleRangeBinding};

/// Core-safe coordinate transform behavior.
///
/// This trait contains the coordinate operations that do not need the
/// top-level chart layout/runtime engine. The top-level chart crate layers its
/// measurement hook on top while facet/concat layout remains core-owned there.
pub trait CoordinateSystemTransformCore: Send + Sync {
    fn required_channels(&self) -> &'static [&'static str];

    /// Transform position channels to coordinate system geometry.
    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError>;

    /// Get the default range binding for a coordinate channel.
    fn default_range_binding(&self, _channel: &str) -> Option<ScaleRangeBinding> {
        None
    }

    /// Get the default range for a coordinate channel.
    fn default_range(
        &self,
        channel: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        self.default_range_binding(channel)
            .and_then(|binding| binding.resolve(plot_area_width, plot_area_height))
    }

    /// Get default scale options for a coordinate channel.
    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue>;
}
