use crate::cartesian::Cartesian;
use crate::define_position_channels;
use crate::impl_mark_trait_common;
use crate::marks::{CompiledDataContext, CompiledMark, CompiledMarkState, Mark, RadiusExpression};
use crate::theme::Theme;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
// Import Symbol for the macro, then re-export it
use crate::channel::ChannelDescriptor;
use crate::coords::CoordinateSystemTransform;
use crate::error::AvengerChartError;
pub use crate::marks::symbol::Symbol;
use crate::render::RenderContext;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// Define position channels for Cartesian Symbol using the macro
define_position_channels! {
    Symbol<Cartesian> {
        x: {
            with_config: crate::cartesian::channels::CartesianPositionConfig,
        },
        y: {
            with_config: crate::cartesian::channels::CartesianPositionConfig,
        }
    }
}

// Implement Mark trait for Cartesian Symbol
#[async_trait::async_trait]
impl Mark<Cartesian> for Symbol<Cartesian> {
    impl_mark_trait_common!(Symbol);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<std::sync::Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(std::sync::Arc::new(CompiledCartesianSymbol {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianSymbol {
    pub(crate) state: CompiledMarkState,
}

#[typetag::serde]
impl CompiledMark for CompiledCartesianSymbol {
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
            // Position channels
            ChannelDescriptor {
                name: "x",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
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

        // Extract position channels based on what the coordinate system requires
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
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "x" | "y" => {
                // Get size and stroke_width expressions (either mapped or default)
                let size_expr = resolve_channel("size");
                let stroke_width_expr = resolve_channel("stroke_width");

                if std::env::var("AVENGER_DEBUG_RADIUS").is_ok() {
                    eprintln!(
                        "DEBUG Symbol: radius_expression called for dimension '{}'",
                        dimension
                    );
                    eprintln!("  size_expr: {:?}", size_expr);
                    eprintln!("  stroke_width_expr: {:?}", stroke_width_expr);
                }

                // For symbols: radius = sqrt(area) * 0.5 + stroke_width / 2 + 4px
                // The size channel represents the area of the bounding square
                // The base circle SVG path has radius 0.5 for a unit square (size=1)
                // Add half the stroke width since stroke extends both inward and outward
                use datafusion::functions::expr_fn::sqrt;
                let radius_expr =
                    sqrt(size_expr) * lit(0.5) + stroke_width_expr / lit(2.0) + lit(4.0);

                use crate::serialization::LogicalExprNodeExt;
                use datafusion_proto::protobuf::LogicalExprNode;
                let radius_expr_node =
                    LogicalExprNode::from_expr(radius_expr).expect("Failed to serialize expr");
                Some(RadiusExpression::Symmetric(radius_expr_node))
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
        crate::marks::symbol::symbol_legend_renderer(channel, scale, &["x", "y", "x2", "y2"])
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
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
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

    fn default_channel_range(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        domain: &crate::scales::ResolvedDomain,
        _data_type: &datafusion::arrow::datatypes::DataType,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
    ) -> Option<crate::scales::ScaleRange> {
        use crate::scales::ScaleRange;
        use datafusion::logical_expr::lit;
        use datafusion_common::ScalarValue;

        // Query theme first, passing cardinality for discrete scales to enable
        // cardinality-specific ranges (e.g., CSS rules like [cardinality="3"])
        let range_kind = scale_impl.range_kind();
        let cardinality = match domain {
            crate::scales::ResolvedDomain::Discrete(count) => Some(*count),
            crate::scales::ResolvedDomain::Interval => None,
        };

        if let Some(theme_range) =
            theme.get_range_for_channel("symbol", channel, range_kind, cardinality, params)
        {
            return Some(theme_range);
        }

        // Provide mark-specific computed defaults for channels with domain-aware logic
        match channel {
            "size" => match domain {
                crate::scales::ResolvedDomain::Discrete(count) => {
                    let min = 40.0;
                    let max = 400.0;
                    if *count == 1 {
                        Some(ScaleRange::new_discrete(vec![ScalarValue::Float32(Some(
                            max,
                        ))]))
                    } else {
                        Some(ScaleRange::new_linspace_discrete(min, max, *count))
                    }
                }
                crate::scales::ResolvedDomain::Interval => {
                    Some(ScaleRange::new_interval(lit(0.0), lit(400.0)))
                }
            },
            "angle" => Some(domain.make_interval_or_linspaced_range(0.0, 360.0)),
            "opacity" => Some(domain.make_interval_or_linspaced_range(0.0, 1.0)),
            "stroke_width" => Some(domain.make_interval_or_linspaced_range(0.5, 5.0)),
            _ => None,
        }
    }
}
