use avenger_scales::scales::ConfiguredScale;
use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AngleChannelConfig, ColorChannelConfig, LegendRendererKind, MarkState, ShapeChannelConfig,
    SizeChannelConfig, StrokeWidthChannelConfig, define_common_mark_channels, impl_mark_base,
    is_continuous_scale,
};

pub struct Symbol<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base!(Symbol);

define_common_mark_channels! {
    Symbol {
        size: {
            with_config: SizeChannelConfig,
        },
        fill: {
            with_config: ColorChannelConfig,
        },
        stroke: {
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            allow_column: false,
            with_config: StrokeWidthChannelConfig,
        },
        shape: {
            with_config: ShapeChannelConfig,
        },
        angle: {
            with_config: AngleChannelConfig,
        },
    }
}

/// Get default values for Symbol mark channels.
pub fn symbol_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "size" => Some(ScalarValue::Float32(Some(64.0))),
        "shape" => Some(ScalarValue::Utf8(Some("circle".to_string()))),
        "angle" => Some(ScalarValue::Float32(Some(0.0))),
        "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        _ => None,
    }
}

/// Get the preferred legend renderer for Symbol marks.
pub fn symbol_legend_renderer_kind(
    channel: &str,
    scale: &ConfiguredScale,
    position_channels: &[&str],
) -> Option<LegendRendererKind> {
    let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

    match channel {
        "fill" | "stroke" | "color" if is_continuous => Some(LegendRendererKind::Colorbar),
        "fill" | "stroke" | "color" | "size" | "shape" | "opacity" | "stroke_width" => {
            Some(LegendRendererKind::Symbol)
        }
        "angle" | "defined" | "order" => None,
        _ => {
            if position_channels.contains(&channel) {
                None
            } else {
                Some(LegendRendererKind::Symbol)
            }
        }
    }
}
