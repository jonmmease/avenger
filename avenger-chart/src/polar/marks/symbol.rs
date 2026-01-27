use crate::define_position_channels;
use crate::error::AvengerChartError;
use crate::impl_mark_trait_common;
use crate::marks::{CompiledDataContext, CompiledMark, CompiledMarkState, Mark, RadiusExpression};
use std::sync::Arc;

use crate::channel::ChannelDescriptor;
use crate::coords::CoordinateSystemTransform;
use crate::polar::Polar;
use crate::render::RenderContext;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::logical_expr::Expr;
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};
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
#[async_trait::async_trait]
impl Mark<Polar> for Symbol<Polar> {
    impl_mark_trait_common!(Symbol);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<std::sync::Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(std::sync::Arc::new(CompiledPolarSymbol {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledPolarSymbol {
    pub(crate) state: CompiledMarkState,
}

// CompiledMark implementation
#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledPolarSymbol {
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
            // Position channels for polar coordinates
            ChannelDescriptor {
                name: "r",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "theta",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
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
        ]
    }

    async fn measure_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &RenderContext,
        _coord: Box<dyn CoordinateSystemTransform>,
    ) -> Result<Box<dyn crate::marks::MarkMeasurement>, AvengerChartError> {
        // Regular marks don't need cached data between measure/render passes
        Ok(Box::new(crate::marks::EmptyMarkMeasurement))
    }

    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: Box<dyn CoordinateSystemTransform>,
        _measurement: &dyn crate::marks::MarkMeasurement,
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
            let value = coerce_numeric_channel_with_renderer(
                self,
                data,
                scalars,
                channel_name,
                context,
                0.0,
            )?;
            position_channels.insert(*channel_name, value);
        }

        // Transform position channels to plot coordinates
        let geometry = coord.transform(
            &position_channels,
            None,
            context.plot_width(),
            context.plot_height(),
        )?;
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

    fn radius_expression(
        &self,
        dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        // For polar coordinates, we don't use radius-aware padding
        // The r and theta channels already account for the polar nature of the plot
        match dimension {
            "r" | "theta" => None,
            _ => None,
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<Arc<dyn crate::legend::LegendRenderer>> {
        // Use the same logic as the Symbol mark
        crate::marks::symbol::symbol_legend_renderer(channel, scale, &["r", "theta"])
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn crate::scales::ScaleSpec>> {
        use crate::scales::spec::{Ordinal, Point, Sqrt};
        use datafusion::arrow::datatypes::DataType;

        match (channel, data_type) {
            // Symbol marks use point scales for categorical position data
            ("r" | "theta", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(Box::new(Point::default()))
            }
            // Size uses sqrt scale for numeric data (better for area perception)
            (
                "size",
                DataType::Float32
                | DataType::Float64
                | DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64,
            ) => {
                // Use Sqrt scale for better area perception
                Some(Box::new(Sqrt::default()))
            }
            // Size uses ordinal for categorical data
            ("size", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(Box::new(Ordinal::default()))
            }
            // Color and shape channels use ordinal scales for categorical data
            (
                "fill" | "stroke" | "color" | "shape",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(Box::new(Ordinal::default())),
            // Stroke width uses ordinal scale only for categorical data
            ("stroke_width", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(Box::new(Ordinal::default()))
            }
            // Fall back to data type-based inference for other channels
            _ => crate::marks::default_scale_for_data_type(data_type),
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> std::collections::HashMap<String, datafusion::logical_expr::Expr> {
        use datafusion::logical_expr::lit;
        use std::collections::HashMap;
        let mut options = HashMap::new();

        // Configure PowScale as Sqrt scale for size channel
        if channel == "size" && scale_impl.scale_type() == "pow" {
            options.insert("exponent".to_string(), lit(0.5f32));
        }

        // For color channels, use the parent implementation
        if matches!(channel, "fill" | "stroke" | "color")
            && crate::marks::util::is_continuous_scale(scale_impl)
        {
            options.insert("nice".to_string(), lit(true));
        }

        options
    }
}
