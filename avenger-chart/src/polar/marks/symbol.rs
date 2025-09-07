use crate::define_position_channels;
use crate::error::AvengerChartError;
use crate::impl_mark_trait_common;
use crate::marks::{Mark, RadiusExpression};

use crate::polar::Polar;
use crate::render_context::RenderContext;
use crate::scales::ScaleRange;
use crate::utils::ScalarValueHelpers;
use arrow::array::RecordBatch;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use datafusion::arrow::datatypes::DataType;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
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
    impl_mark_trait_common!(Symbol, Polar, "symbol");

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "size" => Some(ScalarValue::Float32(Some(64.0))), // Default area
            "shape" => Some(ScalarValue::Utf8(Some("circle".to_string()))), // Default shape
            "angle" => Some(ScalarValue::Float32(Some(0.0))), // Default angle
            "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))), // Default blue
            "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))), // Default black
            "stroke_width" => Some(ScalarValue::Float32(Some(1.0))), // Default stroke width
            "opacity" => Some(ScalarValue::Float32(Some(1.0))), // Fully opaque
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
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use crate::marks::util::{
            coerce_color_channel_with_mark, coerce_numeric_channel_with_mark,
        };
        use avenger_scales::scales::coerce::Coercer;

        // Symbols can render with just scalar data
        let coercer = Coercer::default();

        // Extract polar position data - these can be scalars or arrays
        let r = coerce_numeric_channel_with_mark(self, data, scalars, "r", 0.0)?;
        let theta = coerce_numeric_channel_with_mark(self, data, scalars, "theta", 0.0)?;

        // Get center coordinates from scalar batch if available, otherwise use defaults
        let center_x = if let Some(center_col) = scalars.column_by_name("polar_center_x") {
            if let Some(array) = center_col
                .as_any()
                .downcast_ref::<datafusion::arrow::array::Float32Array>()
            {
                if !array.is_empty() {
                    array.value(0)
                } else {
                    250.0_f32 // fallback
                }
            } else {
                250.0_f32 // fallback
            }
        } else {
            250.0_f32 // fallback for non-dynamic layout
        };

        let center_y = if let Some(center_col) = scalars.column_by_name("polar_center_y") {
            if let Some(array) = center_col
                .as_any()
                .downcast_ref::<datafusion::arrow::array::Float32Array>()
            {
                if !array.is_empty() {
                    array.value(0)
                } else {
                    250.0_f32 // fallback
                }
            } else {
                250.0_f32 // fallback
            }
        } else {
            250.0_f32 // fallback for non-dynamic layout
        };

        // Convert r and theta to x and y
        use avenger_common::value::{ScalarOrArray as SOA, ScalarOrArrayValue};
        let (x, y) = match (r.value(), theta.value()) {
            (ScalarOrArrayValue::Scalar(r_val), ScalarOrArrayValue::Scalar(theta_val)) => {
                // Both are scalars
                let x_val = center_x + r_val * theta_val.cos();
                let y_val = center_y + r_val * theta_val.sin();
                (SOA::new_scalar(x_val), SOA::new_scalar(y_val))
            }
            (ScalarOrArrayValue::Array(r_arr), ScalarOrArrayValue::Array(theta_arr)) => {
                // Both are arrays
                let mut x_values = Vec::with_capacity(r_arr.len());
                let mut y_values = Vec::with_capacity(r_arr.len());

                for i in 0..r_arr.len() {
                    let r_val = r_arr[i];
                    let theta_val = theta_arr[i];
                    x_values.push(center_x + r_val * theta_val.cos());
                    y_values.push(center_y + r_val * theta_val.sin());
                }

                (SOA::new_array(x_values), SOA::new_array(y_values))
            }
            (ScalarOrArrayValue::Scalar(r_val), ScalarOrArrayValue::Array(theta_arr)) => {
                // r is scalar, theta is array
                let mut x_values = Vec::with_capacity(theta_arr.len());
                let mut y_values = Vec::with_capacity(theta_arr.len());

                for theta_val in theta_arr.iter() {
                    x_values.push(center_x + r_val * theta_val.cos());
                    y_values.push(center_y + r_val * theta_val.sin());
                }

                (SOA::new_array(x_values), SOA::new_array(y_values))
            }
            (ScalarOrArrayValue::Array(r_arr), ScalarOrArrayValue::Scalar(theta_val)) => {
                // r is array, theta is scalar
                let mut x_values = Vec::with_capacity(r_arr.len());
                let mut y_values = Vec::with_capacity(r_arr.len());

                for r_val in r_arr.iter() {
                    x_values.push(center_x + r_val * theta_val.cos());
                    y_values.push(center_y + r_val * theta_val.sin());
                }

                (SOA::new_array(x_values), SOA::new_array(y_values))
            }
        };

        // Extract other channels using mark defaults (same as Cartesian)
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

        // Handle shape channel - same as Cartesian
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
                // Array data for shapes
                coercer.to_symbol_shape(shape_array, Some(shape_default))?
            } else if let Some(shape_scalar) = scalars.column_by_name("shape") {
                // Scalar shape
                coercer.to_symbol_shape(shape_scalar, Some(shape_default))?
            } else {
                // Default shape from mark
                (vec![shape_default], ScalarOrArray::new_scalar(0))
            };

        // Stroke width - same as Cartesian
        let stroke_width_default = self
            .default_channel_value("stroke_width", context)
            .and_then(|scalar| scalar.as_f32().ok())
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

        // Create the scene symbol mark
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

    fn default_channel_range(
        &self,
        channel: &str,
        scale_type: &str,
        _data_type: &DataType,
    ) -> Option<ScaleRange> {
        match channel {
            "size" => {
                // Size range depends on scale type
                match scale_type {
                    "linear" | "pow" | "sqrt" => {
                        // For continuous scales, use area range
                        Some(ScaleRange::new_interval(lit(16.0), lit(64.0))) // 4^2 to 8^2
                    }
                    "ordinal" => {
                        // For ordinal scales, create discrete sizes
                        let n = 5; // Default to 5 sizes
                        let sizes: Vec<f32> = (0..n)
                            .map(|i| {
                                let t = if n > 1 {
                                    i as f32 / (n - 1) as f32
                                } else {
                                    0.5
                                };
                                16.0 + t * (64.0 - 16.0) // Interpolate areas from 4^2 to 8^2
                            })
                            .collect();
                        Some(ScaleRange::new_discrete(sizes))
                    }
                    _ => None,
                }
            }
            "shape" => {
                if scale_type == "ordinal" {
                    // Use theme shape sequence
                    let theme = crate::theme::get_default_theme();
                    Some(theme.get_shape_range(None))
                } else {
                    None
                }
            }
            "angle" => Some(ScaleRange::new_interval(lit(0.0), lit(360.0))),
            "opacity" => Some(ScaleRange::new_interval(lit(0.0), lit(1.0))),
            "stroke_width" => {
                if scale_type == "ordinal" {
                    let widths: Vec<f32> = (1..=5).map(|i| i as f32).collect();
                    Some(ScaleRange::new_discrete(widths))
                } else {
                    Some(ScaleRange::new_interval(lit(0.5), lit(5.0)))
                }
            }
            "fill" | "stroke" | "color" => {
                // Use theme color system
                let theme = crate::theme::get_default_theme();
                Some(theme.get_color_range(scale_type, None))
            }
            _ => None,
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend_renderer::LegendRenderer>> {
        use crate::legend_renderer::{ColorbarRenderer, SymbolLegendRenderer};
        use std::sync::Arc;

        // Check if scale is continuous (for colorbar)
        let scale_type = scale.scale_impl.scale_type();
        let is_continuous = matches!(
            scale_type,
            "linear" | "log" | "pow" | "sqrt" | "symlog" | "time"
        );

        match channel {
            // Use colorbar for continuous color scales
            "fill" | "stroke" | "color" if is_continuous => Some(Arc::new(ColorbarRenderer::new())),
            // Symbol marks use symbol legend for discrete scales and other visual properties
            "fill" | "stroke" | "color" | "size" | "shape" | "opacity" | "stroke_width" => {
                Some(Arc::new(SymbolLegendRenderer::new()))
            }
            // No legend for position channels and utility channels
            "r" | "theta" | "defined" | "order" | "angle" => None,
            // For any other channel, default to symbol legend
            _ => Some(Arc::new(SymbolLegendRenderer::new())),
        }
    }
}
