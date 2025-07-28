use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::error::AvengerChartError;
use crate::marks::{ChannelType, Mark, MarkState};
use crate::{
    define_common_mark_channels, define_position_mark_channels, impl_mark_common,
    impl_mark_trait_common,
};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::scalar::ScalarValue;

pub struct Symbol<C: CoordinateSystem> {
    state: MarkState<C>,
    __phantom: std::marker::PhantomData<C>,
}

// Generate common methods
impl_mark_common!(Symbol, "symbol");

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Symbol {
        size: {
            type: ChannelType::Size,
            default: ScalarValue::Float32(Some(64.0))  // Default area
        },
        fill: {
            type: ChannelType::Color,
            default: ScalarValue::Utf8(Some("#4682b4".to_string()))
        },
        stroke: {
            type: ChannelType::Color,
            default: ScalarValue::Utf8(Some("#000000".to_string()))
        },
        stroke_width: {
            type: ChannelType::Size,
            default: ScalarValue::Float32(Some(1.0))
        },
        opacity: {
            type: ChannelType::Numeric,
            default: ScalarValue::Float32(Some(1.0))
        },
        shape: {
            type: ChannelType::Enum { values: &["circle", "square", "cross", "diamond", "triangle-up", "triangle-down", "triangle-left", "triangle-right"] },
            default: ScalarValue::Utf8(Some("circle".to_string()))
        },
    }
}

// Define position channels for Cartesian coordinates
define_position_mark_channels! {
    Symbol<Cartesian> {
        x: { type: ChannelType::Position },
        y: { type: ChannelType::Position },
    }
}

// Define position channels for Polar coordinates
define_position_mark_channels! {
    Symbol<Polar> {
        r: { type: ChannelType::Position },
        theta: { type: ChannelType::Position },
    }
}

// Implement Mark trait for Cartesian Symbol
impl Mark<Cartesian> for Symbol<Cartesian> {
    impl_mark_trait_common!(Symbol, Cartesian, "symbol");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Symbol mark rendering not yet implemented".to_string(),
        ))
    }
}

// Implement Mark trait for Polar Symbol
impl Mark<Polar> for Symbol<Polar> {
    impl_mark_trait_common!(Symbol, Polar, "symbol");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Polar symbol mark rendering not yet implemented".to_string(),
        ))
    }
}
