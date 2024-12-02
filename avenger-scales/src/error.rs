#[derive(Debug, thiserror::Error)]
pub enum AvengerScaleError {
    #[error("Domain length ({domain_len}) does not match range length ({range_len})")]
    DomainRangeMismatch { domain_len: usize, range_len: usize },

    #[error("Empty domain")]
    EmptyDomain,

    #[error("Empty range")]
    EmptyRange,

    #[error("Bins must be in ascending order: {0:?}")]
    BinsNotAscending(Vec<f32>),
}
