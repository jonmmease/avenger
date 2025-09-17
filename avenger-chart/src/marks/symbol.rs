use crate::channel::{
    AngleChannelConfig, ColorChannelConfig, ShapeChannelConfig, SizeChannelConfig,
    StrokeWidthChannelConfig,
};
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::{Mark, MarkState};
use crate::render_context::RenderContext;
use crate::scales::{ResolvedDomain, ScaleRange};
use crate::{define_common_mark_channels, impl_mark_base};
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion_common::ScalarValue;

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
    /// Common rendering logic for symbols in any coordinate system
    ///
    /// This helper extracts position channels, transforms them to plot coordinates,
    /// and builds the scene mark. Used by both Cartesian and Polar implementations.
    pub fn render_from_data_common(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: &C,
    ) -> Result<Vec<SceneMark>, AvengerChartError>
    where
        Self: Mark<C>,
        C: CoordinateSystem<PlotGeometry = crate::coords::PointGeometry>,
    {
        use crate::marks::util::{
            coerce_color_channel_with_mark, coerce_numeric_channel_with_mark,
        };
        use avenger_common::value::ScalarOrArray;
        use avenger_scales::scales::coerce::Coercer;
        use avenger_scenegraph::marks::symbol::SceneSymbolMark;
        use datafusion_common::ScalarValue;

        // Extract position channels based on what the coordinate system requires
        let mut position_channels = std::collections::HashMap::new();
        for channel_name in coord.required_channels() {
            let value =
                coerce_numeric_channel_with_mark(self, data, scalars, channel_name, context, 0.0)?;
            position_channels.insert(*channel_name, value);
        }

        // Transform position channels to plot coordinates
        let geometry =
            coord.transform(&position_channels, context.plot_width, context.plot_height)?;
        let x = geometry.x;
        let y = geometry.y;

        // Extract other channels using mark defaults
        let size = coerce_numeric_channel_with_mark(self, data, scalars, "size", context, 64.0)?;
        let fill = coerce_color_channel_with_mark(
            self,
            data,
            scalars,
            "fill",
            context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_mark(
            self,
            data,
            scalars,
            "stroke",
            context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let angle = coerce_numeric_channel_with_mark(self, data, scalars, "angle", context, 0.0)?;

        // Determine the number of symbols from any array channel
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Handle shape channel efficiently - get default from mark
        let coercer = Coercer::default();
        let shape_default = self
            .default_channel_value("shape", context)
            .and_then(|scalar| {
                match scalar {
                    ScalarValue::Utf8(Some(s)) => {
                        // Convert string to SymbolShape using from_vega_str
                        avenger_common::types::SymbolShape::from_vega_str(&s).ok()
                    }
                    _ => None,
                }
            })
            .unwrap_or(avenger_common::types::SymbolShape::Circle);

        let (shapes, shape_index) =
            if let Some(shape_array) = data.and_then(|d| d.column_by_name("shape")) {
                // Array data for shapes - use efficient coercion
                coercer.to_symbol_shape(shape_array, Some(shape_default))?
            } else if let Some(shape_scalar) = scalars.column_by_name("shape") {
                // Scalar shape - still use to_symbol_shape for consistency
                coercer.to_symbol_shape(shape_scalar, Some(shape_default))?
            } else {
                // Default shape from mark
                (vec![shape_default], ScalarOrArray::new_scalar(0))
            };

        // Stroke width is scalar only - get default from mark
        let stroke_width_scalar = self.default_channel_value("stroke_width", context);

        let stroke_width_default = stroke_width_scalar
            .and_then(|scalar| crate::utils::ScalarValueHelpers::as_f32(&scalar).ok())
            .unwrap_or(1.0);

        let stroke_width = if let Some(width_scalar) = scalars.column_by_name("stroke_width") {
            let val = *coercer
                .to_numeric(width_scalar, Some(stroke_width_default))?
                .first()
                .unwrap();
            Some(val)
        } else {
            Some(stroke_width_default)
        };

        let symbol_mark = SceneSymbolMark {
            name: "symbol".to_string(),
            clip: true,
            len,
            gradients: vec![],
            shapes,
            stroke_width,
            shape_index,
            x,
            y,
            fill,
            size,
            stroke,
            angle,
            indices: None,
            zindex: self.get_zindex(),
            x_adjustment: None,
            y_adjustment: None,
        };

        Ok(vec![SceneMark::Symbol(symbol_mark)])
    }

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
