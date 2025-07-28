use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::error::AvengerChartError;
use crate::marks::{ChannelDescriptor, ChannelType, Mark, MarkConfig};
use crate::{define_common_mark_channels, define_position_mark_channels, impl_mark_common};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::scalar::ScalarValue;
use std::collections::HashMap;

pub struct Symbol<C: CoordinateSystem> {
    config: MarkConfig<C>,
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

// Stub implementations for Mark trait
impl Mark<Cartesian> for Symbol<Cartesian> {
    fn into_config(self) -> MarkConfig<Cartesian> {
        self.config
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        Self::all_channel_descriptors()
    }

    fn render_from_data(
        &self,
        _batch: Option<&RecordBatch>,
        _scalars: &HashMap<String, ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Symbol mark rendering not yet implemented".to_string(),
        ))
    }
}

impl Mark<Polar> for Symbol<Polar> {
    fn into_config(self) -> MarkConfig<Polar> {
        self.config
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        Self::all_channel_descriptors()
    }

    fn render_from_data(
        &self,
        _batch: Option<&RecordBatch>,
        _scalars: &HashMap<String, ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Polar symbol mark rendering not yet implemented".to_string(),
        ))
    }
}
