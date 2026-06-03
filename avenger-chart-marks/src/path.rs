use datafusion_common::ScalarValue;

use avenger_chart_core::{
    ColorChannelConfig, MarkState, OpacityChannelConfig, PathChannelConfig,
    StrokeWidthChannelConfig, TransformChannelConfig, define_common_mark_channels, impl_mark_base,
};

pub struct PathMark<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base!(PathMark);

define_common_mark_channels! {
    PathMark {
        path: {
            allow_column: true,
            with_config: PathChannelConfig,
        },
        transform: {
            allow_column: true,
            with_config: TransformChannelConfig,
        },
        fill: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            allow_column: false,
            with_config: StrokeWidthChannelConfig,
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
    }
}

/// Get default values for PathMark channels.
pub fn path_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "transform" => Some(ScalarValue::Utf8(Some("".to_string()))),
        "fill" => Some(ScalarValue::Utf8(Some("transparent".to_string()))),
        "stroke" => Some(ScalarValue::Utf8(Some("transparent".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(0.0))),
        "stroke_cap" => Some(ScalarValue::Utf8(Some("butt".to_string()))),
        "stroke_join" => Some(ScalarValue::Utf8(Some("miter".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        _ => None,
    }
}
