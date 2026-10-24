use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerAppError {
    #[error("Internal error: `{0}`")]
    InternalError(String),
    /// An app whose scenes, streams, or tools don't fit together.
    #[error("Invalid app: {0}")]
    InvalidApp(String),
}
