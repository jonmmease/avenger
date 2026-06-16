use std::{any::Any, collections::HashMap};

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ScaleImpl;
use datafusion::scalar::ScalarValue as DfScalarValue;
use serde::{Deserialize, Serialize};

use crate::{
    coords::{
        CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
        CoordinateSystemTransformCore, PlotGeometry, SubplotGeometry,
    },
    error::AvengerChartError,
    facet::guide::FacetRowGuideConfig,
};

/// Row faceting coordinate system
///
/// Facets data along the row dimension, creating a vertical stack of subplots.
/// Each subplot represents one unique value from the `row` channel.
///
/// # Example
/// ```ignore
/// let plot = Plot::<FacetRow>::new()
///     .data(df)
///     .mark(
///         Subplot::new(
///             Plot::<Cartesian>::new().mark(Symbol::new()...),
///         )
///         .row(col("species")),
///     );
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct FacetRow;

impl CoordinateSystemCore for FacetRow {
    fn required_channels(&self) -> &'static [&'static str] {
        &["row"]
    }
}

impl CoordinateSystem for FacetRow {
    type Guide = FacetRowGuideConfig;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for FacetRow {
    fn required_channels(&self) -> &'static [&'static str] {
        &["row"]
    }

    fn channel_uses_scale(&self, channel: &str) -> bool {
        channel != "row"
    }

    fn transform(
        &self,
        _position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        Ok(Box::new(SubplotGeometry::default()))
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, DfScalarValue> {
        HashMap::new()
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for FacetRow {
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

    #[test]
    fn facet_row_constructs_struct() {
        let _coord = FacetRow;
    }
}
