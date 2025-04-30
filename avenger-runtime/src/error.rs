use arrow_schema::ArrowError;
use avenger_scales::error::AvengerScaleError;
use datafusion_common::DataFusionError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AvengerRuntimeError {
    #[error("Internal runtime error: {0}")]
    InternalError(String),

    #[error("Variable not found: `{0}`")]
    VariableNotFound(String),

    #[error("Expression not found: `{0}`")]
    ExpressionNotFound(String),

    #[error("Dataset not found: `{0}`")]
    DatasetNotFound(String),

    #[error("Dependency cycle: `{0}`")]
    DependencyCycle(String),

    #[error("Scale error: `{0}`")]
    ScaleError(#[from] AvengerScaleError),

    #[error("DataFusion error: `{0}`")]
    DataFusionError(#[from] DataFusionError),

    #[error("Arrow error: `{0}`")]
    ArrowError(#[from] ArrowError),

    #[error("Tokio join error: `{0}`")]
    TokioJoinError(#[from] tokio::task::JoinError),
}

pub trait DuplicateResult {
    fn duplicate(&self) -> Self;
}

impl<T: Clone> DuplicateResult for Result<T, AvengerRuntimeError> {
    fn duplicate(&self) -> Self {
        match self {
            Ok(v) => Ok(v.clone()),
            Err(err) => Err(err.duplicate()),
        }
    }
}

impl AvengerRuntimeError {
    /// Duplicate an error, returning the same error type if possible
    /// For wrapped error types that don't support cloning, the error is
    /// converted to a generic internal error
    pub fn duplicate(&self) -> Self {
        match self {
            Self::InternalError(e) => Self::InternalError(e.clone()),
            Self::VariableNotFound(e) => Self::VariableNotFound(e.clone()),
            Self::ExpressionNotFound(e) => Self::ExpressionNotFound(e.clone()),
            Self::DatasetNotFound(e) => Self::DatasetNotFound(e.clone()),
            Self::DependencyCycle(e) => Self::DependencyCycle(e.clone()),
            Self::ScaleError(e) => Self::InternalError(e.to_string()),
            Self::DataFusionError(e) => Self::InternalError(e.to_string()),
            Self::ArrowError(e) => Self::InternalError(e.to_string()),
            Self::TokioJoinError(e) => Self::InternalError(e.to_string()),
        }
    }
}

