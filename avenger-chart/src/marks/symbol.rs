use crate::coords::CoordinateSystem;
use crate::marks::{ChannelDefault, ChannelType, MarkState};
use crate::{define_common_mark_channels, impl_mark_base};
use datafusion::scalar::ScalarValue;

pub struct Symbol<C: CoordinateSystem> {
    pub(crate) state: MarkState<C>,
    __phantom: std::marker::PhantomData<C>,
}

// Implement MarkBase trait and Default
impl_mark_base!(Symbol);

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Symbol {
        size: {
            type: ChannelType::Numeric,
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(64.0)))  // Default area
        },
        fill: {
            type: ChannelType::Color,
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("#4682b4".to_string())))
        },
        stroke: {
            type: ChannelType::Color,
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("#000000".to_string())))
        },
        stroke_width: {
            type: ChannelType::Numeric,
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(1.0))),
            allow_column: false  // Symbol stroke width must be constant
        },
        shape: {
            type: ChannelType::SymbolShape,
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("circle".to_string())))
        },
        angle: {
            type: ChannelType::Numeric,
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(0.0)))  // Default: no rotation
        },
    }
}

