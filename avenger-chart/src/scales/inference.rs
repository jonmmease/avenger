//! Scale inference for automatic scale type and option selection

use datafusion::arrow::datatypes::DataType;
use std::collections::HashMap;

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

/// Determine the default scale type based on data type and channel
pub fn infer_scale_type(channel: &str, data_type: &DataType) -> &'static str {
    infer_scale_type_with_mark(channel, data_type, None)
}

/// Determine the default scale type based on data type, channel, and optionally mark type
pub fn infer_scale_type_with_mark(
    channel: &str,
    data_type: &DataType,
    mark_type: Option<&str>,
) -> &'static str {
    match (channel, data_type, mark_type) {
        // Rect marks use band scales for categorical position data
        ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View, Some("rect")) => {
            "band"
        }

        // Other marks use point scales for categorical position data
        ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View, _) => "point",

        // Color and shape channels use ordinal scales for categorical data
        (
            "fill" | "stroke" | "color" | "shape",
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            _,
        ) => "ordinal",

        // Boolean data
        (_, DataType::Boolean, _) => "ordinal",

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
        ) => "linear",

        // Temporal data uses time scale
        (_, DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _), _) => "time",

        // Default to linear for unknown types
        _ => "linear",
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
        ("y", "linear") => {
            options.insert("zero".to_string(), lit(true));
            options.insert("nice".to_string(), lit(true));
        }

        // X-axis linear scales don't necessarily need zero
        ("x", "linear") => {
            options.insert("nice".to_string(), lit(true));
        }

        // Band scales have padding
        (_, "band") => {
            options.insert("padding_inner".to_string(), lit(0.1));
            options.insert("padding".to_string(), lit(0.1));
            options.insert("align".to_string(), lit(0.5));
        }

        // Point scales have padding too
        (_, "point") => {
            options.insert("padding".to_string(), lit(0.5));
            options.insert("align".to_string(), lit(0.5));
        }

        _ => {}
    }

    options
}
