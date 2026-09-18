use crate::SelectionId;

/// Invalid definitions, updates, mappings, queries, or dataflow bindings.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid selection definition: {0}")]
    InvalidDefinition(String),
    #[error("Invalid selection value: {0}")]
    InvalidValue(String),
    #[error("Selection {0} is not defined")]
    MissingSelection(SelectionId),
    #[error("Selection {0} is defined more than once")]
    DuplicateSelection(SelectionId),
    #[error("Invalid selection update: {0}")]
    InvalidUpdate(String),
    #[error("Invalid consumer mapping: {0}")]
    InvalidMapping(String),
    #[error("Invalid selection query: {0}")]
    InvalidQuery(String),
    #[cfg(feature = "dataflow")]
    #[error(transparent)]
    Dataflow(#[from] avenger_datafusion_dataflow::Error),
    #[error(transparent)]
    DataFusion(#[from] datafusion::common::DataFusionError),
}

/// A result from selection construction, updates, resolution, or query binding.
pub type Result<T> = std::result::Result<T, Error>;
