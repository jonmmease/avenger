use crate::define_position_channels;
use crate::impl_mark_trait_common;
use crate::marks::{Mark, RadiusExpression};

use crate::polar::Polar;
use crate::render::RenderContext;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;

// Import Line for the macro, then re-export it
use crate::error::AvengerChartError;
use crate::marks::line::Line;

// Define position channels for Polar Line using the macro
define_position_channels! {
    Line<Polar> {
        r: {
            with_config: crate::polar::channels::PolarPositionConfig,
        },
        theta: {
            with_config: crate::polar::channels::PolarPositionConfig,
        }
    }
}

// Implement Mark trait for PolarGeneral Line with any axis type
impl Mark<Polar> for Line<Polar> {
    impl_mark_trait_common!(Line, Polar, "line");

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))), // Default black
            "stroke_width" => Some(ScalarValue::Float32(Some(2.0))),          // Default line width
            "stroke_cap" => Some(ScalarValue::Utf8(Some("round".to_string()))), // Default cap style
            "stroke_join" => Some(ScalarValue::Utf8(Some("round".to_string()))), // Default join style
            "opacity" => Some(ScalarValue::Float32(Some(1.0))),                  // Fully opaque
            "interpolate" => Some(ScalarValue::Utf8(Some("linear".to_string()))), // Linear interpolation
            "defined" => Some(ScalarValue::Boolean(Some(true))), // All points defined
            _ => None,
        }
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "r" => {
                // Get stroke_width expression (either mapped or default)
                let stroke_width_expr = resolve_channel("stroke_width");

                // For lines: radial radius = stroke_width * 2
                let radius_expr = stroke_width_expr * lit(2.0);

                Some(RadiusExpression::Symmetric(radius_expr))
            }
            "theta" => {
                // No angular radius for line marks
                None
            }
            _ => None,
        }
    }

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &RenderContext,
        _coord: &Polar,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Polar line mark rendering not yet implemented".to_string(),
        ))
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend::LegendRenderer>> {
        use crate::legend::{ColorbarRenderer, LineLegendRenderer};
        use std::sync::Arc;

        // Check if scale is continuous (for colorbar)
        let is_continuous = crate::marks::util::is_continuous_scale(scale.scale_impl.as_ref());

        match channel {
            // Use colorbar for continuous color scales
            "stroke" if is_continuous => Some(Arc::new(ColorbarRenderer::new())),
            // Line marks use line legend for discrete scales and line-specific properties
            "stroke" | "stroke_width" | "stroke_dash" => Some(Arc::new(LineLegendRenderer::new())),
            // No legend for position channels and other non-visual channels
            "r" | "theta" | "r2" | "theta2" | "defined" | "order" => None,
            // For any other channel, default to line legend
            _ => Some(Arc::new(LineLegendRenderer::new())),
        }
    }
}
