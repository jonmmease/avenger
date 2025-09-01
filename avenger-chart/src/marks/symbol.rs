use crate::channel_configs::{
    AngleChannelConfig, ColorChannelConfig, ShapeChannelConfig, SizeChannelConfig,
    StrokeWidthChannelConfig,
};
use crate::coords::CoordinateSystem;
use crate::marks::{ChannelDefault, MarkState};
use crate::{define_common_mark_channels, impl_mark_base};
use datafusion::scalar::ScalarValue;

pub struct Symbol<C: CoordinateSystem> {
    pub(crate) state: MarkState<C>,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

// Implement MarkBase trait and Default
impl_mark_base!(Symbol);

// Define common channels using the macro with explicit config types
define_common_mark_channels! {
    Symbol {
        size: {
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(64.0))),
            with_config: SizeChannelConfig,
        },
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
            allow_column: false,
            with_config: StrokeWidthChannelConfig,
        },
        shape: {
            default: ChannelDefault::Scalar(ScalarValue::Utf8(Some("circle".to_string()))),
            with_config: ShapeChannelConfig,
        },
        angle: {
            default: ChannelDefault::Scalar(ScalarValue::Float32(Some(0.0))),
            with_config: AngleChannelConfig,
        },
    }
}
