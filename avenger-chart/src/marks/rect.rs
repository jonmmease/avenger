use crate::channel_configs::{ColorChannelConfig, OpacityChannelConfig, StrokeWidthChannelConfig};
use crate::coords::CoordinateSystem;
use crate::marks::{ChannelDefault, ChannelType, MarkState};
use crate::{define_common_mark_channels, impl_mark_base};
use datafusion::scalar::ScalarValue;

pub struct Rect<C: CoordinateSystem> {
    pub(crate) state: MarkState<C>,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

// Implement MarkBase trait and Default
impl_mark_base!(Rect);

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Rect {
        fill: {
            type: ChannelType::Color,
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("#4682b4".to_string()))),
            with_config: ColorChannelConfig
        },
        stroke: {
            type: ChannelType::Color,
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("#000000".to_string()))),
            with_config: ColorChannelConfig
        },
        stroke_width: {
            type: ChannelType::Numeric,
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(1.0))),
            with_config: StrokeWidthChannelConfig
        },
        opacity: {
            type: ChannelType::Numeric,
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(1.0))),
            with_config: OpacityChannelConfig
        },
        corner_radius: {
            type: ChannelType::Numeric,
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(0.0)))
        },
    }
}
