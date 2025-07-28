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

pub struct Line<C: CoordinateSystem> {
    state: MarkState<C>,
    __phantom: std::marker::PhantomData<C>,
}

// Generate common methods
impl_mark_common!(Line, "line");

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Line {
        stroke: {
            type: ChannelType::Color,
            default: ScalarValue::Utf8(Some("#000000".to_string()))
        },
        stroke_width: {
            type: ChannelType::Size,
            default: ScalarValue::Float32(Some(2.0)),
            allow_column: false  // Line width must be constant
        },
        stroke_dash: {
            type: ChannelType::Enum { values: &["solid", "dashed", "dotted"] },
            default: ScalarValue::Utf8(Some("solid".to_string())),
            allow_column: false
        },
        opacity: {
            type: ChannelType::Numeric,
            default: ScalarValue::Float32(Some(1.0))
        },
        interpolate: {
            type: ChannelType::Enum { values: &["linear", "step", "step-before", "step-after", "basis", "cardinal", "monotone"] },
            default: ScalarValue::Utf8(Some("linear".to_string())),
            allow_column: false
        },
    }
}

// Define position channels for Cartesian coordinates
define_position_mark_channels! {
    Line<Cartesian> {
        x: { type: ChannelType::Position },
        y: { type: ChannelType::Position },
    }
}

// Define position channels for Polar coordinates
define_position_mark_channels! {
    Line<Polar> {
        r: { type: ChannelType::Position },
        theta: { type: ChannelType::Position },
    }
}

// Implement Mark trait for Cartesian Line
impl Mark<Cartesian> for Line<Cartesian> {
    impl_mark_trait_common!(Line, Cartesian, "line");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Line mark rendering not yet implemented".to_string(),
        ))
    }
}

// Implement Mark trait for Polar Line
impl Mark<Polar> for Line<Polar> {
    impl_mark_trait_common!(Line, Polar, "line");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Polar line mark rendering not yet implemented".to_string(),
        ))
    }
}
