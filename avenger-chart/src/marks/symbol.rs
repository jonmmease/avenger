use crate::channel::{
    AngleChannelConfig, ColorChannelConfig, ShapeChannelConfig, SizeChannelConfig,
    StrokeWidthChannelConfig,
};
use crate::coords::CoordinateSystem;
use crate::marks::MarkState;
use crate::{define_common_mark_channels, impl_mark_base};
use datafusion_common::ScalarValue;

pub struct Symbol<C: CoordinateSystem> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

// Implement MarkBase trait and Default
impl_mark_base!(Symbol);

// Define common channels using the macro with explicit config types
define_common_mark_channels! {
    Symbol {
        size: {
            // Default now comes from theme
            with_config: SizeChannelConfig,
        },
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
            allow_column: false,
            with_config: StrokeWidthChannelConfig,
        },
        shape: {
            // Default now comes from theme
            with_config: ShapeChannelConfig,
        },
        angle: {
            // Default now comes from theme
            with_config: AngleChannelConfig,
        },
    }
}

// Free functions for Symbol renderer implementations to share common logic

/// Get default values for Symbol mark channels
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

/// Get the preferred legend renderer for Symbol marks
pub fn symbol_legend_renderer(
    channel: &str,
    scale: &avenger_scales::scales::ConfiguredScale,
    position_channels: &[&str],
) -> Option<std::sync::Arc<dyn crate::legend::LegendRenderer>> {
    use crate::legend::{ColorbarRenderer, SymbolLegendRenderer};
    use crate::marks::util::is_continuous_scale;
    use std::sync::Arc;

    let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

    match channel {
        "fill" | "stroke" | "color" if is_continuous => Some(Arc::new(ColorbarRenderer::new())),
        "fill" | "stroke" | "color" | "size" | "shape" | "opacity" | "stroke_width" => {
            Some(Arc::new(SymbolLegendRenderer::new()))
        }
        "angle" | "defined" | "order" => None,
        _ => {
            if position_channels.contains(&channel) {
                None
            } else {
                Some(Arc::new(SymbolLegendRenderer::new()))
            }
        }
    }
}
