use arrow::array::RecordBatch;
use datafusion::logical_expr::{lit, Expr};
use datafusion_common::ScalarValue;
use avenger_scenegraph::marks::mark::SceneMark;
use crate::{define_position_mark_channels, impl_mark_trait_common};
use crate::marks::{ChannelType, Mark, RadiusExpression};
use crate::polar::Polar;

// Import Line for the macro, then re-export it
use crate::marks::line::Line;
use crate::error::AvengerChartError;

// Define position channels for Polar Line
define_position_mark_channels! {
    Line<Polar> {
        r: { type: ChannelType::Numeric },
        theta: { type: ChannelType::Numeric },
    }
}

// Implement Mark trait for Polar Line
impl Mark<Polar> for Line<Polar> {
    impl_mark_trait_common!(Line, Polar, "line");

    fn default_channel_value(&self, channel: &str) -> Option<ScalarValue> {
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
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Polar line mark rendering not yet implemented".to_string(),
        ))
    }
}


