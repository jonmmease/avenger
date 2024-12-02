#[derive(Debug, PartialEq, thiserror::Error)]
pub enum AvengerScaleError {
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
}
