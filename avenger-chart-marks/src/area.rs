use datafusion_common::ScalarValue;

use avenger_chart_core::{
    ColorChannelConfig, MarkState, OpacityChannelConfig, StrokeDashChannelConfig,
    StrokeWidthChannelConfig, define_common_mark_channels, impl_mark_base,
};

pub struct Area<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base!(Area);

define_common_mark_channels! {
    Area {
        orientation: {
            allow_column: false,
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

/// Partitioning key for area marks with varying scalar style fields.
#[doc(hidden)]
#[derive(Hash, Eq, PartialEq, Debug, Clone, Ord, PartialOrd)]
pub struct AreaPartitionKey {
    pub fill: Option<usize>,
    pub stroke: Option<usize>,
    pub width: Option<usize>,
    pub dash: Option<usize>,
    pub opacity: Option<usize>,
}

/// Get default values for Area mark channels.
pub fn area_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "orientation" => Some(ScalarValue::Utf8(Some("vertical".to_string()))),
        "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
        "stroke" => Some(ScalarValue::Utf8(Some("transparent".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(0.0))),
        "stroke_cap" => Some(ScalarValue::Utf8(Some("butt".to_string()))),
        "stroke_join" => Some(ScalarValue::Utf8(Some("round".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "defined" => Some(ScalarValue::Boolean(Some(true))),
        _ => None,
    }
}
