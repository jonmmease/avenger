use crate::channel_configs::{
    AngleChannelConfig, ColorChannelConfig, ShapeChannelConfig, SizeChannelConfig,
    StrokeWidthChannelConfig,
};
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::{Mark, MarkState};
use crate::render_context::RenderContext;
use crate::{define_common_mark_channels, impl_mark_base};
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;

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
            let value = coerce_numeric_channel_with_mark(self, data, scalars, channel_name, 0.0)?;
            position_channels.insert(*channel_name, value);
        }

        // Transform position channels to plot coordinates
        let (x, y) = coord.transform_to_plot_coords(
            &position_channels,
            context.plot_width,
            context.plot_height,
        )?;

        // Extract other channels using mark defaults
        let size = coerce_numeric_channel_with_mark(self, data, scalars, "size", 64.0)?;
        let fill = coerce_color_channel_with_mark(
            self,
            data,
            scalars,
            "fill",
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke =
            coerce_color_channel_with_mark(self, data, scalars, "stroke", [0.0, 0.0, 0.0, 1.0])?;
        let angle = coerce_numeric_channel_with_mark(self, data, scalars, "angle", 0.0)?;

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
        let stroke_width_default = self
            .default_channel_value("stroke_width", context)
            .and_then(|scalar| crate::utils::ScalarValueHelpers::as_f32(&scalar).ok())
            .unwrap_or(1.0);

        let stroke_width = if let Some(width_scalar) = scalars.column_by_name("stroke_width") {
            Some(
                *coercer
                    .to_numeric(width_scalar, Some(stroke_width_default))?
                    .first()
                    .unwrap(),
            )
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
}
