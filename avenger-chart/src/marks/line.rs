use crate::channel::{
    ColorChannelConfig, OpacityChannelConfig, StrokeDashChannelConfig, StrokeWidthChannelConfig,
};
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::MarkState;
use crate::{define_common_mark_channels, impl_mark_base};
use datafusion::arrow::array::ArrayRef;
use datafusion::arrow::compute::kernels::cast::cast;
use datafusion::arrow::datatypes::DataType;
use datafusion_common::ScalarValue;

pub struct Line<C: CoordinateSystem> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

// Implement MarkBase trait and Default
impl_mark_base!(Line);

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Line {
        stroke: {
            // Default now comes from theme
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            // Default now comes from theme
            allow_column: true,
            with_config: StrokeWidthChannelConfig,
        },
        stroke_dash: {
            // Default now comes from theme
            allow_column: true,
            with_config: StrokeDashChannelConfig,
        },
        stroke_cap: {
            // Default now comes from theme
            allow_column: false,
        },
        stroke_join: {
            // Default now comes from theme
            allow_column: false,
        },
        opacity: {
            // Default now comes from theme
            allow_column: false,
            with_config: OpacityChannelConfig,
        },
        defined: {
            // Default now comes from theme
        },
        order: {
            allow_column: true,
        },
    }
}

/// Partitioning key for multi-series lines
#[derive(Hash, Eq, PartialEq, Debug, Clone, Ord, PartialOrd)]
pub(crate) struct PartitionKey {
    pub stroke: Option<usize>,
    pub width: Option<usize>,
    pub dash: Option<usize>,
}

/// Convert an array to dictionary encoding for efficient partitioning
pub fn ensure_dictionary_array(array: &ArrayRef) -> Result<ArrayRef, AvengerChartError> {
    match array.data_type() {
        DataType::Dictionary(_, _) => Ok(array.clone()),
        _ => {
            // Convert to dictionary for efficient partitioning
            let dict_type = DataType::Dictionary(
                Box::new(DataType::Int16),
                Box::new(array.data_type().clone()),
            );
            Ok(cast(array, &dict_type)?)
        }
    }
}

/// Get default values for Line mark channels
pub fn line_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(2.0))),
        "stroke_cap" => Some(ScalarValue::Utf8(Some("round".to_string()))),
        "stroke_join" => Some(ScalarValue::Utf8(Some("round".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "interpolate" => Some(ScalarValue::Utf8(Some("linear".to_string()))),
        "defined" => Some(ScalarValue::Boolean(Some(true))),
        _ => None,
    }
}
