use avenger_scales::scales::{RangeKind, ScaleImpl};
use datafusion::arrow::datatypes::DataType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScaleTypePreference {
    Linear,
    Log,
    Pow,
    Sqrt,
    Symlog,
    Time,
    Band,
    NestedBand,
    Point,
    Ordinal,
    Threshold,
    Quantile,
    Quantize,
}

/// Check if a scale represents a continuous scale based on its range kind.
///
/// A scale is considered continuous if it produces continuous numeric output
/// in its range, regardless of its domain type.
pub fn is_continuous_scale(scale_impl: &dyn ScaleImpl) -> bool {
    scale_impl.range_kind() == RangeKind::Continuous
}

/// Default scale type inference based on data type alone.
///
/// This function can be called by marks that override preferred scale type to
/// provide fallback behavior for unhandled channels.
pub fn default_scale_type_for_data_type(data_type: &DataType) -> Option<ScaleTypePreference> {
    match data_type {
        // Categorical data uses ordinal scale
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Boolean => {
            Some(ScaleTypePreference::Ordinal)
        }
        // Temporal data uses time scale
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _) => {
            Some(ScaleTypePreference::Time)
        }
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
        | DataType::UInt64 => Some(ScaleTypePreference::Linear),
        // Default to None for unknown types
        _ => None,
    }
}
