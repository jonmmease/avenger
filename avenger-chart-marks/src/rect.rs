use datafusion_common::ScalarValue;

use avenger_chart_core::{
    ColorChannelConfig, MarkState, OpacityChannelConfig, StrokeWidthChannelConfig,
    define_common_mark_channels, impl_mark_base,
};

pub struct Rect<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base!(Rect);

define_common_mark_channels! {
    Rect {
        fill: {
            with_config: ColorChannelConfig,
        },
        stroke: {
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            with_config: StrokeWidthChannelConfig,
        },
        opacity: {
            with_config: OpacityChannelConfig,
        },
        corner_radius: {},
    }
}

/// Get default values for Rect mark channels.
pub fn rect_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
        "corner_radius" => Some(ScalarValue::Float32(Some(0.0))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        _ => None,
    }
}
