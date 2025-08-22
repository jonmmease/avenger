use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::{ChannelDefault, ChannelType, MarkState};
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
            type: ChannelType::Color,
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("#000000".to_string()))),
            allow_column: true
        },
        stroke_width: {
            type: ChannelType::Numeric,
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(2.0))),
            allow_column: true
        },
        stroke_dash: {
            type: ChannelType::StrokeDash,
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("solid".to_string()))),
            allow_column: true
        },
        stroke_cap: {
            type: ChannelType::StrokeCap,
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("round".to_string()))),
            allow_column: false
        },
        stroke_join: {
            type: ChannelType::StrokeJoin,
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("round".to_string()))),
            allow_column: false
        },
        opacity: {
            type: ChannelType::Numeric,
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(1.0))),
            allow_column: false  // Line opacity must be constant
        },
        defined: {
            type: ChannelType::Boolean,
            default: ChannelDefault::Scalar(ScalarValue::Boolean(Some(true)))
        },
        order: {
            type: ChannelType::Numeric,
            allow_column: true
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
