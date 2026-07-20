use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererSelection, Mark,
    MarkRuntimeContext, PointGeometry, RadiusExpression, ScalarValueHelpers, ScaleTypePreference,
    coerce_color_channel_with_renderer, coerce_numeric_channel_with_renderer,
    default_scale_type_for_data_type, impl_mark_trait_common, is_continuous_scale,
    serialization::DefaultLogicalExprNodeExt,
};
use avenger_chart_marks::{Symbol, symbol_channel_defaults, symbol_legend_renderer_kind};
use avenger_common::{types::SymbolShape, value::ScalarOrArray};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, coerce::Coercer};
use avenger_scenegraph::marks::{
    mark::SceneMark, pattern::default_no_fill_pattern, symbol::SceneSymbolMark,
};
use datafusion::{
    arrow::{array::RecordBatch, datatypes::DataType as ArrowDataType},
    common::ScalarValue,
    functions::expr_fn::sqrt,
    logical_expr::{Expr, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};

use super::super::Polar;

// Implement Mark trait for PolarGeneral Symbol with any axis type
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Polar> for Symbol<Polar> {
    impl_mark_trait_common!(Symbol);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        if !self.mark_effects().is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "Symbol<Polar> adjustments are not implemented yet".to_string(),
            ));
        }
        Ok(Arc::new(CompiledPolarSymbol {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledPolarSymbol {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledPolarSymbol {
    avenger_chart_core::impl_mark_with_data_context!();

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

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        symbol_channel_defaults(channel)
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            // Radial scale padding keeps a symbol at the outer inferred
            // radius inside the circular plot boundary. Theta wraps around
            // the circle and therefore does not need endpoint padding.
            "r" => {
                let size_expr = resolve_channel("size");
                let stroke_width_expr = resolve_channel("stroke_width");
                let radius_expr =
                    sqrt(size_expr) * lit(0.5) + stroke_width_expr / lit(2.0) + lit(4.0);
                let radius_expr_node = LogicalExprNode::from_default_expr(radius_expr)
                    .expect("Failed to serialize Polar symbol radius expr");
                Some(RadiusExpression::Symmetric(radius_expr_node))
            }
            _ => None,
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        // Use the same logic as the Symbol mark
        symbol_legend_renderer_kind(channel, scale, &["r", "theta"])
            .map(LegendRendererSelection::BuiltIn)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &ArrowDataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            // Symbol marks use point scales for categorical position data
            (
                "r" | "theta",
                ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View,
            ) => Some(ScaleTypePreference::Point),
            // Size uses sqrt scale for numeric data (better for area perception)
            (
                "size",
                ArrowDataType::Float32
                | ArrowDataType::Float64
                | ArrowDataType::Int8
                | ArrowDataType::Int16
                | ArrowDataType::Int32
                | ArrowDataType::Int64
                | ArrowDataType::UInt8
                | ArrowDataType::UInt16
                | ArrowDataType::UInt32
                | ArrowDataType::UInt64,
            ) => {
                // Use Sqrt scale for better area perception
                Some(ScaleTypePreference::Sqrt)
            }
            // Size uses ordinal for categorical data
            ("size", ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            // Color and shape channels use ordinal scales for categorical data
            (
                "fill" | "stroke" | "color" | "shape",
                ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View,
            ) => Some(ScaleTypePreference::Ordinal),
            // Stroke width uses ordinal scale only for categorical data
            (
                "stroke_width",
                ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View,
            ) => Some(ScaleTypePreference::Ordinal),
            // Fall back to data type-based inference for other channels
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _data_type: &ArrowDataType,
    ) -> HashMap<String, Expr> {
        let mut options = HashMap::new();

        // Configure PowScale as Sqrt scale for size channel
        if channel == "size" && scale_impl.scale_type() == "pow" {
            options.insert("exponent".to_string(), lit(0.5f32));
        }

        // For color channels, use the parent implementation
        if matches!(channel, "fill" | "stroke" | "color") && is_continuous_scale(scale_impl) {
            options.insert("nice".to_string(), lit(true));
        }

        options
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledPolarSymbol {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mark_context = context.core_view();

        // Extract position channels for polar coordinates
        let mut position_channels = HashMap::new();
        for channel_name in coord.required_channels() {
            let value = coerce_numeric_channel_with_renderer(
                self,
                data,
                scalars,
                channel_name,
                &mark_context,
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
            fill_pattern: default_no_fill_pattern(),
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
