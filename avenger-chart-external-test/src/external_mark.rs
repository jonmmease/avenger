//! Integration test to verify external marks can be defined and used

use avenger_chart::error::AvengerChartError;
use avenger_chart::{
    cartesian::Cartesian,
    coords::CoordinateSystem,
    define_common_mark_channels, define_position_channels, impl_mark_base, impl_mark_trait_common,
    marks::{Mark, MarkState},
};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::scalar::ScalarValue;
use std::marker::PhantomData;

/// A custom hexbin mark defined in an external crate
pub struct HexBin<C: CoordinateSystem> {
    state: MarkState<C>,
    _phantom: PhantomData<C>,
}

// Use the exported macro for base implementation
impl_mark_base!(HexBin);

// Define common channels
define_common_mark_channels! {
    HexBin {
        fill: {},
        stroke: {},
        opacity: {},
        size: {},
    }
}

// Define position channels for Cartesian HexBin using the macro
define_position_channels! {
    HexBin<Cartesian> {
        x: {
            required: true,
            with_config: avenger_chart::cartesian::channels::CartesianPositionConfig,
        },
        y: {
            required: true,
            with_config: avenger_chart::cartesian::channels::CartesianPositionConfig,
        }
    }
}

// Implement the Mark trait for Cartesian
impl Mark<Cartesian> for HexBin<Cartesian> {
    impl_mark_trait_common!(HexBin, Cartesian, "hexbin");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Custom hexbin rendering logic would go here
        // For this test, we just return an empty vector
        Ok(vec![])
    }

    fn default_channel_value(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "size" => Some(ScalarValue::Float32(Some(20.0))),
            "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
            _ => None,
        }
    }
}
