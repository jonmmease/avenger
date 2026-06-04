//! Mark implementations for ZeroDCoord
//!
//! Only Symbol marks are supported in zero-dimensional coordinate systems.
//! Line and Rect marks don't make sense without spatial extent.

use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererSelection, Mark,
    MarkRuntimeContext, PointGeometry, ScalarValueHelpers, ZeroDCoord,
    coerce_color_channel_with_renderer, coerce_numeric_channel_with_renderer,
    impl_mark_trait_common,
};
use avenger_common::{types::SymbolShape, value::ScalarOrArray};
use avenger_scales::scales::{ConfiguredScale, coerce::Coercer};
use avenger_scenegraph::marks::{mark::SceneMark, symbol::SceneSymbolMark};
use datafusion::{arrow::array::RecordBatch, common::ScalarValue, prelude::SessionContext};
use serde::{Deserialize, Serialize};

use crate::symbol::{Symbol, symbol_channel_defaults, symbol_legend_renderer_kind};

// Implement Mark trait for ZeroDCoord Symbol
#[async_trait::async_trait]
impl Mark<ZeroDCoord> for Symbol<ZeroDCoord> {
    impl_mark_trait_common!(Symbol);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledZeroDSymbol {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledZeroDSymbol {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledZeroDSymbol {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "symbol"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            // No position channels for 0D
            // Common channels
            ChannelDescriptor {
                name: "size",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "fill",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "shape",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "angle",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        symbol_channel_defaults(channel)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        // Use the same logic as the Symbol mark, with no position channels
        symbol_legend_renderer_kind(channel, scale, &[]).map(LegendRendererSelection::BuiltIn)
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledZeroDSymbol {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mark_context = context.core_view();

        // In 0D space, there are no position channels
        // Transform will give us a center point
        let position_channels = HashMap::new();
        let geometry = coord.transform(
            &position_channels,
            None,
            context.plot_width(),
            context.plot_height(),
        )?;
        let geometry = geometry
            .as_any()
            .downcast_ref::<PointGeometry>()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast to PointGeometry".to_string(),
                )
            })?;

        let x = geometry.x.clone();
        let y = geometry.y.clone();

        // Extract other channels using mark defaults
        let size =
            coerce_numeric_channel_with_renderer(self, data, scalars, "size", &mark_context, 64.0)?;
        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            &mark_context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let angle =
            coerce_numeric_channel_with_renderer(self, data, scalars, "angle", &mark_context, 0.0)?;

        // Determine the number of symbols
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Handle shape channel
        let coercer = Coercer::default();
        let shape_default = self
            .default_channel_value("shape", &mark_context)
            .and_then(|scalar| match scalar {
                ScalarValue::Utf8(Some(s)) => SymbolShape::from_vega_str(&s).ok(),
                _ => None,
            })
            .unwrap_or(SymbolShape::Circle);

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
            .default_channel_value("stroke_width", &mark_context)
            .and_then(|scalar| ScalarValueHelpers::as_f32(&scalar).ok())
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
            interactive: true,
        };

        Ok(vec![SceneMark::Symbol(symbol_mark)])
    }
}
