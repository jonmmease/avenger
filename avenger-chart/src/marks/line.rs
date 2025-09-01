use crate::channel_configs::{
    ColorChannelConfig, OpacityChannelConfig, StrokeDashChannelConfig, StrokeWidthChannelConfig,
};
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::{ChannelDefault, MarkState};
use crate::{define_common_mark_channels, impl_mark_base};
use datafusion::arrow::array::ArrayRef;
use datafusion::arrow::compute::kernels::cast::cast;
use datafusion::arrow::datatypes::DataType;
use datafusion::scalar::ScalarValue;

pub struct Line<C: CoordinateSystem> {
    pub(crate) state: MarkState<C>,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

// Implement MarkBase trait and Default
impl_mark_base!(Line);

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Line {
        stroke: {
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("#000000".to_string()))),
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(2.0))),
            allow_column: true,
            with_config: StrokeWidthChannelConfig,
        },
        stroke_dash: {
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("solid".to_string()))),
            allow_column: true,
            with_config: StrokeDashChannelConfig,
        },
        stroke_cap: {
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("round".to_string()))),
            allow_column: false,
        },
        stroke_join: {
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("round".to_string()))),
            allow_column: false,
        },
        opacity: {
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(1.0))),
            allow_column: false,
            with_config: OpacityChannelConfig,
        },
        defined: {
            default: ChannelDefault::Scalar(ScalarValue::Boolean(Some(true))),
        },
        order: {
            allow_column: true,
        },
    }
}

// Position channels are now defined in coordinate-specific modules:
// - cartesian/marks/line.rs for Cartesian
// - polar/marks/line.rs for Polar

// Partitioning support for multi-series lines
#[derive(Hash, Eq, PartialEq, Debug, Clone, Ord, PartialOrd)]
pub struct PartitionKey {
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
