use avenger_scales::scales::{RangeKind, ScaleImpl};
use datafusion::arrow::datatypes::DataType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// Internal scale inference override supplied by compound marks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScaleInferenceHint {
    pub scale_name: String,
    pub preference: ScaleTypePreference,
}

impl ScaleInferenceHint {
    pub fn new(scale_name: impl Into<String>, preference: ScaleTypePreference) -> Self {
        Self {
            scale_name: scale_name.into(),
            preference,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_inference_hint_round_trips() {
        let hint = ScaleInferenceHint::new("y", ScaleTypePreference::Band);
        let json = serde_json::to_string(&hint).expect("serialize json");
        let restored_json: ScaleInferenceHint =
            serde_json::from_str(&json).expect("deserialize json");
        assert_eq!(restored_json, hint);

        let bytes = bincode::serialize(&hint).expect("serialize bincode");
        let restored_bincode: ScaleInferenceHint =
            bincode::deserialize(&bytes).expect("deserialize bincode");
        assert_eq!(restored_bincode, hint);
    }
}
