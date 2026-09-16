use datafusion::arrow::error::ArrowError;
use datafusion::common::DataFusionError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(feature = "json")]
    #[error("invalid JSON dataflow or query: {0}")]
    Json(String),
    #[error("invalid dataflow artifact: {0}")]
    Artifact(String),
    #[error("duplicate {namespace} name: {name}")]
    DuplicateName {
        namespace: &'static str,
        name: String,
    },
    #[error("handle belongs to another graph")]
    ForeignHandle,
    #[error("reference is outside its defining scope: {0}")]
    OutOfScope(String),
    #[error("invalid partition key: {0}")]
    InvalidKey(String),
    #[error("unsupported partition key type: {0}")]
    UnsupportedKeyType(String),
    #[error("invalid scope address: {0}")]
    InvalidScopeAddress(String),
    #[error("scope was not requested")]
    UnrequestedScope,
    #[error("at {instance}: {source}")]
    Scoped {
        instance: crate::ScopeInstance,
        #[source]
        source: Box<Error>,
    },
    #[error("invalid graph reference: {0}")]
    InvalidReference(String),
    #[error("unregistered scalar placeholder: {0}")]
    UnknownPlaceholder(String),
    #[error("invalid standalone expression: {0}")]
    InvalidExpression(String),
    #[error("unsupported plan: {0}")]
    UnsupportedPlan(String),
    #[error("schema mismatch for {0}")]
    SchemaMismatch(String),
    #[error("scalar type mismatch for {name}: expected {expected}, received {actual}")]
    ScalarTypeMismatch {
        name: String,
        expected: String,
        actual: String,
    },
    #[error("missing input: {0}")]
    MissingInput(String),
    #[error("output was not requested")]
    UnrequestedOutput,
    #[error("invalid runtime configuration: {0}")]
    InvalidConfig(String),
    #[error("active materialization exceeds the {limit} byte budget")]
    ResourceExhausted { limit: usize },
    #[error("execution of node '{node}' failed: {source}")]
    Execution {
        node: String,
        #[source]
        source: DataFusionError,
    },
    #[error(transparent)]
    DataFusion(#[from] DataFusionError),
    #[error(transparent)]
    Arrow(#[from] ArrowError),
}
