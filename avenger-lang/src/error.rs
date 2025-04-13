use datafusion_common::DataFusionError;
use thiserror::Error;
use sqlparser::tokenizer::TokenizerError;
use sqlparser::parser::ParserError;


#[derive(Error, Debug)]
pub enum AvengerLangError {
    #[error("Internal error: `{0}`")]
    InternalError(String),

    #[error("Variable not found: `{0}`")]
    VariableNotFound(String),

    #[error("Expression not found: `{0}`")]
    ExpressionNotFound(String),

    #[error("Dataset not found: `{0}`")]
    DatasetNotFound(String),

    #[error("Tokenization error: `{0}`")]
    TokenizerError(#[from] TokenizerError),

    #[error("Parser error: `{0}`")]
    ParserError(#[from] ParserError),

    #[error("DataFusion error: `{0}`")]
    DataFusionError(#[from] DataFusionError),

    #[error("Unexpected token: `{0}`")]
    UnexpectedToken(String),

    #[error("Dependency cycle detected in task graph")]
    DependencyCycle(String),
    
    #[error("Runtime error: `{0}`")]
    RuntimeError(String),
    
    #[error("Dependency error: `{0}`")]
    DependencyError(String),
    
    #[error("Evaluation error: `{0}`")]
    EvaluationError(String),

    #[error("Tokio join error: `{0}`")]
    TokioJoinError(#[from] tokio::task::JoinError),
}
