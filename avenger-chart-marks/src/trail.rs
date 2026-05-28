use datafusion_common::ScalarValue;

use avenger_chart_core::{
    ColorChannelConfig, MarkState, OpacityChannelConfig, SizeChannelConfig,
    define_common_mark_channels, impl_mark_base,
};

pub struct Trail<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base!(Trail);

define_common_mark_channels! {
    Trail {
        size: {
            allow_column: true,
            with_config: SizeChannelConfig,
        },
        stroke: {
            allow_column: true,
            with_config: ColorChannelConfig,
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

/// Partitioning key for trail marks with varying scalar style fields.
#[doc(hidden)]
#[derive(Hash, Eq, PartialEq, Debug, Clone, Ord, PartialOrd)]
pub struct TrailPartitionKey {
    pub stroke: Option<usize>,
    pub opacity: Option<usize>,
}

/// Get default values for Trail mark channels.
pub fn trail_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "size" => Some(ScalarValue::Float32(Some(1.0))),
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "defined" => Some(ScalarValue::Boolean(Some(true))),
        _ => None,
    }
}
