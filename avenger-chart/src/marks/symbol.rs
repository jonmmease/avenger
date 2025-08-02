use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::error::AvengerChartError;
use crate::marks::{ChannelType, Mark, MarkState, RadiusExpression};
use crate::{
    define_common_mark_channels, define_position_mark_channels, impl_mark_common,
    impl_mark_trait_common,
};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, lit};
use datafusion::scalar::ScalarValue;

pub struct Symbol<C: CoordinateSystem> {
    state: MarkState<C>,
    __phantom: std::marker::PhantomData<C>,
}

// Generate common methods
impl_mark_common!(Symbol, "symbol");

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Symbol {
        size: {
            type: ChannelType::Size,
            default: ScalarValue::Float32(Some(64.0))  // Default area
        },
        fill: {
            type: ChannelType::Color,
            default: ScalarValue::Utf8(Some("#4682b4".to_string()))
        },
        stroke: {
            type: ChannelType::Color,
            default: ScalarValue::Utf8(Some("#000000".to_string()))
        },
        stroke_width: {
            type: ChannelType::Size,
            default: ScalarValue::Float32(Some(1.0)),
            allow_column: false  // Symbol stroke width must be constant
        },
        shape: {
            type: ChannelType::Enum { values: &["circle", "square", "cross", "diamond", "triangle-up", "triangle-down", "triangle-left", "triangle-right"] },
            default: ScalarValue::Utf8(Some("circle".to_string()))
        },
        angle: {
            type: ChannelType::Numeric,
            default: ScalarValue::Float32(Some(0.0))  // Default: no rotation
        },
    }
}

// Define position channels for Cartesian coordinates
define_position_mark_channels! {
    Symbol<Cartesian> {
        x: { type: ChannelType::Position },
        y: { type: ChannelType::Position },
    }
}

// Define position channels for Polar coordinates
define_position_mark_channels! {
    Symbol<Polar> {
        r: { type: ChannelType::Position },
        theta: { type: ChannelType::Position },
    }
}

// Implement Mark trait for Cartesian Symbol
impl Mark<Cartesian> for Symbol<Cartesian> {
    impl_mark_trait_common!(Symbol, Cartesian, "symbol");

    fn padding_channels(&self) -> Vec<&'static str> {
        vec!["size", "stroke_width", "shape", "angle"]
    }

    fn default_channel_value(&self, channel: &str) -> Option<Expr> {
        match channel {
            "size" => Some(lit(64.0)),        // Default area
            "shape" => Some(lit("circle")),   // Default shape
            "angle" => Some(lit(0.0)),        // Default angle
            "fill" => Some(lit("#4682b4")),   // Default blue
            "stroke" => Some(lit("#000000")), // Default blac
            "stroke_width" => Some(lit(1.0)), // Default stroke width
            "opacity" => Some(lit(1.0)),      // Fully opaque
            _ => None,
        }
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
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use crate::marks::util::{
            coerce_color_channel_with_mark, coerce_numeric_channel_with_mark,
        };
        use avenger_scales::scales::coerce::Coercer;

        // Symbols can render with just scalar data
        let coercer = Coercer::default();

        // Extract position data - these can be scalars or arrays
        let x = coerce_numeric_channel_with_mark(self, data, scalars, "x", 0.0)?;
        let y = coerce_numeric_channel_with_mark(self, data, scalars, "y", 0.0)?;

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
        let shape_default = self
            .default_channel_value("shape")
            .and_then(|expr| {
                if let datafusion::logical_expr::Expr::Literal(scalar, _) = expr {
                    match scalar {
                        datafusion::scalar::ScalarValue::Utf8(Some(s)) => {
                            // Convert string to SymbolShape using from_vega_str
                            avenger_common::types::SymbolShape::from_vega_str(&s).ok()
                        }
                        _ => None,
                    }
                } else {
                    None
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
            .default_channel_value("stroke_width")
            .and_then(|expr| {
                if let datafusion::logical_expr::Expr::Literal(scalar, _) = expr {
                    match scalar {
                        datafusion::scalar::ScalarValue::Float32(Some(v)) => Some(v),
                        datafusion::scalar::ScalarValue::Float64(Some(v)) => Some(v as f32),
                        datafusion::scalar::ScalarValue::Int32(Some(v)) => Some(v as f32),
                        datafusion::scalar::ScalarValue::Int64(Some(v)) => Some(v as f32),
                        _ => None,
                    }
                } else {
                    None
                }
            })
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
            zindex: self.state.zindex,
            x_adjustment: None,
            y_adjustment: None,
        };

        Ok(vec![SceneMark::Symbol(symbol_mark)])
    }
}

// Implement Mark trait for Polar Symbol
impl Mark<Polar> for Symbol<Polar> {
    impl_mark_trait_common!(Symbol, Polar, "symbol");

    fn default_channel_value(&self, channel: &str) -> Option<Expr> {
        match channel {
            "size" => Some(lit(64.0)), // Default area (matches define_common_mark_channels)
            "shape" => Some(lit("circle")), // Default shape
            "angle" => Some(lit(0.0)), // Default angle
            "fill" => Some(lit("#4682b4")), // Default blue
            "stroke" => Some(lit("#000000")), // Default black (matches define_common_mark_channels)
            "stroke_width" => Some(lit(1.0)), // Default stroke width (matches define_common_mark_channels)
            "opacity" => Some(lit(1.0)),      // Fully opaque
            _ => None,
        }
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
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Polar symbol mark rendering not yet implemented".to_string(),
        ))
    }
}
