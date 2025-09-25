use crate::channel::{
    AngleChannelConfig, ColorChannelConfig, ShapeChannelConfig, SizeChannelConfig,
    StrokeWidthChannelConfig,
};
use crate::coords::{CoordinateSystem, CoordinateSystemTransform};
use crate::error::AvengerChartError;
use crate::marks::{Mark, MarkState};
use crate::render::RenderContext;
use crate::scales::{ResolvedDomain, ScaleRange};
use crate::{define_common_mark_channels, impl_mark_base};
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::logical_expr::UserDefinedLogicalNode;
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

impl<C: CoordinateSystem> Symbol<C> {
    /// Common mark-specific defaults for Symbol marks across all coordinate systems
    pub fn common_mark_specific_default(channel: &str) -> Option<ScalarValue> {
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

    /// Common default channel range for Symbol marks
    pub fn common_default_channel_range(
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        domain: &crate::scales::ResolvedDomain,
        theme: &dyn crate::theme::Theme,
    ) -> Option<ScaleRange> {
        match channel {
            "size" => Some(domain.make_interval_or_linspaced_range(16.0, 64.0)),
            "shape" => {
                // Only provide shapes for categorical domains
                if let ResolvedDomain::Discrete(_) = domain {
                    Some(theme.get_shape_range(None))
                } else {
                    None
                }
            }
            "angle" => Some(domain.make_interval_or_linspaced_range(0.0, 360.0)),
            "opacity" => Some(domain.make_interval_or_linspaced_range(0.0, 1.0)),
            "stroke_width" => Some(domain.make_interval_or_linspaced_range(0.5, 5.0)),
            "fill" | "stroke" | "color" => {
                // Use theme color system
                let range_kind = scale_impl.range_kind();
                Some(theme.get_range_for_channel("symbol", channel, range_kind, None))
            }
            _ => None,
        }
    }

    /// Common preferred legend renderer logic for Symbol marks
    pub fn common_preferred_legend_renderer(
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
}
