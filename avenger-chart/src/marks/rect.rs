use datafusion_common::ScalarValue;

use crate::{
    channel::{ColorChannelConfig, OpacityChannelConfig, StrokeWidthChannelConfig},
    coords::CoordinateSystem,
    marks::MarkState,
    {define_common_mark_channels, impl_mark_base},
};

pub struct Rect<C: CoordinateSystem> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

// Implement MarkBase trait and Default
impl_mark_base!(Rect);

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Rect {
        fill: {
            // Default now comes from theme
            with_config: ColorChannelConfig,
        },
        stroke: {
            // Default now comes from theme
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            // Default now comes from theme
            with_config: StrokeWidthChannelConfig,
        },
        opacity: {
            // Default now comes from theme
            with_config: OpacityChannelConfig,
        },
        corner_radius: {
            allow_column: false,
        },
    }
}

/// Get default values for Rect mark channels
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
