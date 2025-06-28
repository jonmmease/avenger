use crate::scalar::Scalar;
use arrow::error::ArrowError;
use avenger_image::error::AvengerImageError;

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

    #[error("Invalid timezone: {0}")]
    InvalidTimezone(String),

    #[error("Invalid SVG transform string: {0}")]
    InvalidSvgTransformString(#[from] svgtypes::Error),

    #[error("Invalid SVG path string: {0}")]
    InvalidSvgPathString(#[from] lyon_extra::parser::ParseError),

    #[error("Expected scalar value of type {expected_type}, got {scalar:?}")]
    InvalidScaleConfigScalarValue {
        expected_type: String,
        scalar: Scalar,
    },

    #[error("Invalid {data_type} format string: {format_str}")]
    InvalidFormatString {
        format_str: String,
        data_type: String,
    },

    #[error("Avenger image error: {0}")]
    AvengerImageError(#[from] AvengerImageError),

    #[error("Arrow error: {0}")]
    ArrowError(#[from] ArrowError),
}
