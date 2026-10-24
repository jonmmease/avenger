use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerAppError {
    #[error("Internal error: `{0}`")]
    InternalError(String),
    /// Invalid scene or stream configuration.
    #[error("Invalid app: {0}")]
    InvalidApp(String),
}
