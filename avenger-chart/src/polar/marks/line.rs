use crate::impl_mark_trait_common;
use crate::marks::{ChannelType, Mark, RadiusExpression};

use crate::polar::Polar;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;

// Import Line for the macro, then re-export it
use crate::error::AvengerChartError;
use crate::marks::line::Line;

// Implement position channels for PolarGeneral Line with generic axis support
impl Line<Polar> {
    pub fn r<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("r", value.into())
    }

    pub fn r_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::marks::typed_channels::PositionChannel,
        ) -> crate::marks::typed_channels::PositionChannel,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = f(crate::marks::typed_channels::PositionChannel(channel_value));
        self.with_channel_value("r", channel.into())
    }

    pub fn theta<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("theta", value.into())
    }

    pub fn theta_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::marks::typed_channels::PositionChannel,
        ) -> crate::marks::typed_channels::PositionChannel,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = f(crate::marks::typed_channels::PositionChannel(channel_value));
        self.with_channel_value("theta", channel.into())
    }

    pub fn position_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        use crate::marks::ChannelDescriptor;
        vec![
            ChannelDescriptor {
                name: "r",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "theta",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    pub fn all_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        let mut descriptors = Self::common_channel_descriptors();
        descriptors.extend(Self::position_channel_descriptors());
        descriptors
    }
}

// Implement Mark trait for PolarGeneral Line with any axis type
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
