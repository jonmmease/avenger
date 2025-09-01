use crate::channel_configs::{ColorChannelConfig, OpacityChannelConfig, StrokeWidthChannelConfig};
use crate::coords::CoordinateSystem;
use crate::marks::{ChannelDefault, MarkState};
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
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("#4682b4".to_string()))),
            with_config: ColorChannelConfig,
        },
        stroke: {
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("#000000".to_string()))),
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(1.0))),
            with_config: StrokeWidthChannelConfig,
        },
        opacity: {
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(1.0))),
            with_config: OpacityChannelConfig,
        },
        corner_radius: {
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(0.0))),
        },
    }
}
