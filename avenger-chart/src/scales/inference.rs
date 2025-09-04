//! Scale inference for automatic scale type and option selection

use avenger_scales::scales::ScaleImpl;
use avenger_scales::scales::{
    linear::LinearScale, ordinal::OrdinalScale, point::PointScale, time::TimeScale,
};
use datafusion::arrow::datatypes::DataType;
use std::sync::Arc;

/// Determine the default scale implementation for position channels based on data type
/// Position channels default to Point scales for categorical data
pub fn infer_position_scale_impl(data_type: &DataType) -> Arc<dyn ScaleImpl> {
    match data_type {
        // String/categorical data uses point scale for positions
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => Arc::new(PointScale),

        // Boolean data uses point scale for positions
        DataType::Boolean => Arc::new(PointScale),

        // Numeric data defaults to linear
        DataType::Float32
        | DataType::Float64
        | DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => Arc::new(LinearScale),

        // Temporal data uses time scale
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _) => Arc::new(TimeScale),

        // Default to linear for unknown types
        _ => Arc::new(LinearScale),
    }
}

/// Determine the default scale implementation for non-position channels based on data type
/// Non-position channels default to Ordinal scales for categorical data
pub fn infer_scale_impl(data_type: &DataType) -> Arc<dyn ScaleImpl> {
    match data_type {
        // String/categorical data uses ordinal scale for non-positions
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => Arc::new(OrdinalScale),

        // Boolean data uses ordinal scale
        DataType::Boolean => Arc::new(OrdinalScale),

        // Numeric data defaults to linear
        DataType::Float32
        | DataType::Float64
        | DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => Arc::new(LinearScale),

        // Temporal data uses time scale
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _) => Arc::new(TimeScale),

        // Default to linear for unknown types
        _ => Arc::new(LinearScale),
    }
}
