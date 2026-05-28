use datafusion::arrow::{array::ArrayRef, compute::kernels::cast::cast, datatypes::DataType};
use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AvengerChartError, ColorChannelConfig, MarkState, OpacityChannelConfig,
    StrokeDashChannelConfig, StrokeWidthChannelConfig, define_common_mark_channels, impl_mark_base,
};

pub struct Line<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base!(Line);

define_common_mark_channels! {
    Line {
        stroke: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            allow_column: true,
            with_config: StrokeWidthChannelConfig,
        },
        stroke_dash: {
            allow_column: true,
            with_config: StrokeDashChannelConfig,
        },
        stroke_cap: {
            allow_column: false,
        },
        stroke_join: {
            allow_column: false,
        },
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
        defined: {},
        order: {
            allow_column: true,
        },
    }
}

/// Partitioning key for multi-series lines.
#[doc(hidden)]
#[derive(Hash, Eq, PartialEq, Debug, Clone, Ord, PartialOrd)]
pub struct PartitionKey {
    pub stroke: Option<usize>,
    pub width: Option<usize>,
    pub dash: Option<usize>,
    pub opacity: Option<usize>,
}

/// Convert an array to dictionary encoding for efficient partitioning.
pub fn ensure_dictionary_array(array: &ArrayRef) -> Result<ArrayRef, AvengerChartError> {
    match array.data_type() {
        DataType::Dictionary(_, _) => Ok(array.clone()),
        _ => {
            let dict_type = DataType::Dictionary(
                Box::new(DataType::Int16),
                Box::new(array.data_type().clone()),
            );
            Ok(cast(array, &dict_type)?)
        }
    }
}

/// Get default values for Line mark channels.
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
