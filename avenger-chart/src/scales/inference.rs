//! Scale inference for automatic scale type and option selection

use avenger_scales::scales::ScaleImpl;
use avenger_scales::scales::{
    band::BandScale, linear::LinearScale, ordinal::OrdinalScale, point::PointScale, time::TimeScale,
};
use datafusion::arrow::datatypes::DataType;
use std::collections::HashMap;
use std::sync::Arc;

/// Trait for marks to provide their own scale type and option preferences
pub trait MarkScaleInference: Send + Sync {
    /// Get the preferred scale type for a channel based on data type
    fn preferred_scale_type(&self, channel: &str, data_type: &DataType) -> Option<&'static str> {
        // Default implementation returns None, letting the system use defaults
        let _ = (channel, data_type);
        None
    }

    /// Get default scale options for a channel
    fn default_scale_options(
        &self,
        channel: &str,
        scale_type: &str,
        data_type: &DataType,
    ) -> HashMap<String, datafusion::logical_expr::Expr> {
        let _ = (channel, scale_type, data_type);
        HashMap::new()
    }
}

/// Determine the default scale implementation based on data type and channel
pub fn infer_scale_impl(channel: &str, data_type: &DataType) -> Arc<dyn ScaleImpl> {
    infer_scale_impl_with_mark(channel, data_type, None)
}

/// Determine the default scale implementation based on data type, channel, and optionally mark type
pub fn infer_scale_impl_with_mark(
    channel: &str,
    data_type: &DataType,
    mark_type: Option<&str>,
) -> Arc<dyn ScaleImpl> {
    match (channel, data_type, mark_type) {
        // Rect marks use band scales for categorical position data
        ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View, Some("rect")) => {
            Arc::new(BandScale)
        }

        // Other marks use point scales for categorical position data
        ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View, _) => {
            Arc::new(PointScale)
        }

        // Color, shape, size, and dash channels use ordinal scales for categorical data
        (
            "fill" | "stroke" | "color" | "shape" | "size" | "stroke_dash",
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            _,
        ) => Arc::new(OrdinalScale),

        // Stroke width defaults to ordinal scale for discrete mapping
        // (user can override with scale_stroke_width)
        ("stroke_width", _, _) => Arc::new(OrdinalScale),

        // Boolean data
        (_, DataType::Boolean, _) => Arc::new(OrdinalScale),

        // Numeric data defaults to linear
        (
            _,
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
            _,
        ) => Arc::new(LinearScale),

        // Temporal data uses time scale
        (_, DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _), _) => {
            Arc::new(TimeScale)
        }

        // Default to linear for unknown types
        _ => Arc::new(LinearScale),
    }
}

/// Get default scale options based on channel and scale type
pub fn get_default_scale_options(
    channel: &str,
    scale_type: &str,
    _data_type: &DataType,
) -> HashMap<String, datafusion::logical_expr::Expr> {
    use datafusion::logical_expr::lit;
    let mut options = HashMap::new();

    match (channel, scale_type) {
        // Y-axis linear scales typically include zero
        ("y" | "y2", "linear") => {
            options.insert("zero".to_string(), lit(true));
            options.insert("nice".to_string(), lit(true));
            options.insert("round".to_string(), lit(true)); // Pixel-aligned for crisp grid lines
        }

        // X-axis linear scales don't necessarily need zero
        ("x" | "x2", "linear") => {
            options.insert("nice".to_string(), lit(true));
            options.insert("round".to_string(), lit(true)); // Pixel-aligned for crisp grid lines
        }

        // For any numeric positional scale (not just linear), enable rounding for pixel alignment
        ("x" | "x2" | "y" | "y2", "log" | "pow" | "sqrt" | "symlog" | "time") => {
            options.insert("round".to_string(), lit(true)); // Pixel-aligned positions
        }

        // Color scales with numeric data should use nice for better legend labels
        ("fill" | "stroke" | "color", "linear" | "log" | "pow" | "sqrt" | "symlog") => {
            options.insert("nice".to_string(), lit(true));
        }

        // Band scales have padding (already have round by default)
        (_, "band") => {
            options.insert("padding_inner".to_string(), lit(0.1));
            options.insert("padding".to_string(), lit(0.1));
            options.insert("align".to_string(), lit(0.5));
        }

        // Point scales have padding too (already have round by default)
        (_, "point") => {
            options.insert("padding".to_string(), lit(0.5));
            options.insert("align".to_string(), lit(0.5));
        }

        _ => {}
    }

    options
}
