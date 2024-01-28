use thiserror::Error;

#[derive(Error, Debug)]
pub enum SceneGraphError {
    #[error("Internal error: `{0}`")]
    InternalError(String),
}
