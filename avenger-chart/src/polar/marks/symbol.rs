use std::sync::Arc;
use crate::define_position_channels;
use crate::error::AvengerChartError;
use crate::impl_mark_trait_common;
use crate::impl_supported_channels;
use crate::marks::{DataContext, Mark, MarkRenderer, MarkState, RadiusExpression};

use crate::polar::Polar;
use crate::render::RenderContext;
use crate::scales::ScaleRange;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::datatypes::DataType;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};
use crate::cartesian::Cartesian;
use crate::channel::ChannelDescriptor;
use crate::coords::CoordinateSystemTransform;
// Import Symbol for the macro, then re-export it
use crate::marks::symbol::Symbol;

// Define position channels for Polar Symbol using the macro
define_position_channels! {
    Symbol<Polar> {
        r: {
            with_config: crate::polar::channels::PolarPositionConfig,
        },
        theta: {
            with_config: crate::polar::channels::PolarPositionConfig,
        }
    }
}

// Implement Mark trait for PolarGeneral Symbol with any axis type
impl Mark<Polar> for Symbol<Polar> {
    impl_mark_trait_common!(Symbol, "symbol");

    fn build(&self) -> std::sync::Arc<dyn MarkRenderer> {
        std::sync::Arc::new(PolarSymbol {
            state: self.state.clone()
        })
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        Symbol::<Polar>::common_mark_specific_default(channel)
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "r" | "theta" => {
                // Get size and stroke_width expressions (either mapped or default)
                let size_expr = resolve_channel("size");
                let stroke_width_expr = resolve_channel("stroke_width");

                // For symbols: radius = sqrt(area) * 0.5 + stroke_width / 2
                // The size channel represents the area of the bounding square
                // The base circle SVG path has radius 0.5 for a unit square (size=1)
                // Add half the stroke width since stroke extends both inward and outward
                use datafusion::functions::expr_fn::sqrt;
                let radius_expr = sqrt(size_expr) * lit(0.5) + stroke_width_expr / lit(2.0);

                Some(RadiusExpression::Symmetric(radius_expr))
            }
            _ => None,
        }
    }

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: &Polar,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        self.render_from_data_common(data, scalars, context, coord)
    }

    fn default_channel_range(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        domain: &crate::scales::ResolvedDomain,
        _data_type: &DataType,
        theme: &dyn crate::theme::Theme,
    ) -> Option<ScaleRange> {
        Symbol::<Polar>::common_default_channel_range(channel, scale_impl, domain, theme)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend::LegendRenderer>> {
        Symbol::<Polar>::common_preferred_legend_renderer(channel, scale, &["r", "theta"])
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PolarSymbol {
    pub(crate) state: MarkState,
}

impl From<Symbol<Polar>> for PolarSymbol {
    fn from(line: Symbol<Polar>) -> Self {
        Self { state: line.state }
    }
}

// MarkRenderer implementation
#[typetag::serde]
impl MarkRenderer for PolarSymbol {
    fn state(&self) -> &MarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    fn data_context(&self) -> &DataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "symbol"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![]  // TODO: Implement channel descriptors properly
    }

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: Box<dyn CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use crate::marks::util::{
            coerce_color_channel_with_renderer, coerce_numeric_channel_with_renderer,
        };
        use avenger_common::value::ScalarOrArray;
        use avenger_scales::scales::coerce::Coercer;
        use avenger_scenegraph::marks::symbol::SceneSymbolMark;
        use datafusion_common::ScalarValue;

        // Extract position channels for polar coordinates
        let mut position_channels = std::collections::HashMap::new();
        for channel_name in coord.required_channels() {
            let value =
                coerce_numeric_channel_with_renderer(self, data, scalars, channel_name, context, 0.0)?;
            position_channels.insert(*channel_name, value);
        }

        // Transform position channels to plot coordinates
        let geometry =
            coord.transform(&position_channels, context.plot_width, context.plot_height)?;
        let geometry = geometry.as_any().downcast_ref::<crate::coords::PointGeometry>().ok_or_else(
            || AvengerChartError::CoordinateSystemError("Failed to downcast to PointGeometry".to_string())
        )?;

        let x = geometry.x.clone();
        let y = geometry.y.clone();

        // Extract other channels using mark defaults
        let size = coerce_numeric_channel_with_renderer(self, data, scalars, "size", context, 64.0)?;
        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let angle = coerce_numeric_channel_with_renderer(self, data, scalars, "angle", context, 0.0)?;

        // Determine the number of symbols
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Handle shape channel
        let coercer = Coercer::default();
        let shape_default = self
            .default_channel_value("shape", context)
            .and_then(|scalar| {
                match scalar {
                    ScalarValue::Utf8(Some(s)) => {
                        avenger_common::types::SymbolShape::from_vega_str(&s).ok()
                    }
                    _ => None,
                }
            })
            .unwrap_or(avenger_common::types::SymbolShape::Circle);

        let (shapes, shape_index) =
            if let Some(shape_array) = data.and_then(|d| d.column_by_name("shape")) {
                coercer.to_symbol_shape(shape_array, Some(shape_default))?
            } else if let Some(shape_scalar) = scalars.column_by_name("shape") {
                coercer.to_symbol_shape(shape_scalar, Some(shape_default))?
            } else {
                (vec![shape_default], ScalarOrArray::new_scalar(0))
            };

        // Stroke width
        let stroke_width_default = self
            .default_channel_value("stroke_width", context)
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
            zindex: self.state.zindex,
            x_adjustment: None,
            y_adjustment: None,
        };

        Ok(vec![SceneMark::Symbol(symbol_mark)])
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        Symbol::<Polar>::common_mark_specific_default(channel)
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "r" | "theta" => {
                let size_expr = resolve_channel("size");
                let stroke_width_expr = resolve_channel("stroke_width");

                use datafusion::functions::expr_fn::sqrt;
                let radius_expr = sqrt(size_expr) * lit(0.5) + stroke_width_expr / lit(2.0);

                Some(RadiusExpression::Symmetric(radius_expr))
            }
            _ => None,
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<Arc<dyn crate::legend::LegendRenderer>> {
        // Use the same logic as the Symbol mark
        Symbol::<Polar>::common_preferred_legend_renderer(
            channel,
            scale,
            &["r", "theta"],
        )
    }
}
