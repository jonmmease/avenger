//! Mark implementations for ZeroDCoord
//!
//! Only Symbol marks are supported in zero-dimensional coordinate systems.
//! Line and Rect marks don't make sense without spatial extent.

use crate::channel::ChannelDescriptor;
use crate::coords::CoordinateSystemTransform;
use crate::error::AvengerChartError;
use crate::impl_mark_trait_common;
use crate::marks::{CompiledDataContext, CompiledMark, CompiledMarkState, Mark};
use crate::render::RenderContext;
use crate::zerod::ZeroDCoord;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};
// Only Symbol marks are supported in ZeroDCoord
pub use crate::marks::symbol::Symbol;
// Implement position channel methods for ZeroDCoord marks
// Since ZeroDCoord has no position channels (0D space), these implementations are minimal

impl Symbol<ZeroDCoord> {
    pub fn position_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        vec![] // No position channels in 0D space
    }

    pub fn all_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        let mut descriptors = Self::common_channel_descriptors();
        descriptors.extend(Self::position_channel_descriptors());
        descriptors
    }
}

// Implement Mark trait for ZeroDCoord Symbol
impl Mark<ZeroDCoord> for Symbol<ZeroDCoord> {
    impl_mark_trait_common!(Symbol, CompiledZeroDSymbol);
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledZeroDSymbol {
    pub(crate) state: CompiledMarkState,
}

// CompiledMark implementation
#[typetag::serde]
impl CompiledMark for CompiledZeroDSymbol {
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

    fn evaluate_from_data(
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

        // In 0D space, there are no position channels
        // Transform will give us a center point
        let position_channels = std::collections::HashMap::new();
        let geometry =
            coord.transform(&position_channels, context.plot_width, context.plot_height)?;
        let geometry = geometry
            .as_any()
            .downcast_ref::<crate::coords::PointGeometry>()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast to PointGeometry".to_string(),
                )
            })?;

        let x = geometry.x.clone();
        let y = geometry.y.clone();

        // Extract other channels using mark defaults
        let size =
            coerce_numeric_channel_with_renderer(self, data, scalars, "size", context, 64.0)?;
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
        let angle =
            coerce_numeric_channel_with_renderer(self, data, scalars, "angle", context, 0.0)?;

        // Determine the number of symbols
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Handle shape channel
        let coercer = Coercer::default();
        let shape_default = self
            .default_channel_value("shape", context)
            .and_then(|scalar| match scalar {
                ScalarValue::Utf8(Some(s)) => {
                    avenger_common::types::SymbolShape::from_vega_str(&s).ok()
                }
                _ => None,
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
        crate::marks::symbol::symbol_channel_defaults(channel)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend::LegendRenderer>> {
        // Use the same logic as the Symbol mark, with no position channels
        crate::marks::symbol::symbol_legend_renderer(channel, scale, &[])
    }
}
