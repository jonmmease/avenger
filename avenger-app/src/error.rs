use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerAppError {
    #[error("Internal error: `{0}`")]
    InternalError(String),
}
