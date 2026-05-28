use datafusion_common::ScalarValue;

use avenger_chart_core::{
    ColorChannelConfig, MarkState, OpacityChannelConfig, StrokeDashChannelConfig,
    StrokeWidthChannelConfig, define_common_mark_channels, impl_mark_base,
};

pub struct Rule<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base!(Rule);

define_common_mark_channels! {
    Rule {
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
            allow_column: true,
        },
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
    }
}

/// Get default values for Rule mark channels.
pub fn rule_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
        "stroke_cap" => Some(ScalarValue::Utf8(Some("butt".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        _ => None,
    }
}
