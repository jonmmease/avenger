use arrow::error::ArrowError;
use datafusion_common::{DataFusionError, ScalarValue};

#[derive(Debug, thiserror::Error)]
pub enum AvengerScaleError {
    #[error("Internal error: {0}")]
    InternalError(String),

    #[error("Domain length ({domain_len}) does not match range length ({range_len})")]
    DomainRangeMismatch { domain_len: usize, range_len: usize },

    #[error("Empty domain")]
    EmptyDomain,

    #[error("Empty range")]
    EmptyRange,

    #[error("Bins must be in ascending order: {0:?}")]
    BinsNotAscending(Vec<f32>),

    #[error("Thresholds must be in ascending order: {0:?}")]
    ThresholdsNotAscending(Vec<f32>),

    #[error("Domain value not found: {0}")]
    DomainValueNotFound(String),

    #[error(
        "Threshold domain length ({domain_len}) must be one less than range length ({range_len})"
    )]
    ThresholdDomainMismatch { domain_len: usize, range_len: usize },

    #[error("Incompatible numeric scale for color range: {0}")]
    IncompatibleNumericScaleForColorRange(String),

    #[error("Invalid scale property value: {0}")]
    InvalidScalePropertyValue(String),

    #[error("Config not compatible with scale type: {0}")]
    IncompatibleConfig(String),

    #[error("Scale operation not supported for this scale type: {0}")]
    ScaleOperationNotSupported(String),

    #[error("Expected scalar value of type {expected_type}, got {scalar:?}")]
    InvalidScaleConfigScalarValue {
        expected_type: String,
        scalar: ScalarValue,
    },

    #[error("Invalid {data_type} format string: {format_str}")]
    InvalidFormatString {
        format_str: String,
        data_type: String,
    },

    #[error("Arrow error: {0}")]
    ArrowError(#[from] ArrowError),

    #[error("DataFusion error: {0}")]
    DataFusionError(#[from] DataFusionError),
}
