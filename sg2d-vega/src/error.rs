use thiserror::Error;

#[derive(Error, Debug)]
pub enum VegaSceneGraphError {
    #[error("SceneGraph error")]
    SceneGraphError(#[from] sg2d::error::SceneGraphError),

    #[error("css color parse error")]
    InvalidColor(#[from] csscolorparser::ParseColorError),
}
