use thiserror::Error;

#[derive(Error, Debug)]
pub enum VegaSceneGraphError {
    #[error("SceneGraph error")]
    SceneGraphError(#[from] sg2d::error::SceneGraphError),

    #[error("css color parse error")]
    InvalidColor(#[from] csscolorparser::ParseColorError),

    #[error("image error")]
    ImageError(#[from] image::ImageError),

    // ParseError doesn't implement std::Error, so #[from] doesn't seem to work
    #[error("Error parsing SVG path")]
    InvalidSvgPath(lyon_extra::parser::ParseError),

    #[error("Invalid dash string: {0}")]
    InvalidDashString(String),

    #[error("Image fetching is not enabled: {0}")]
    NoImageFetcherConfigured(String),

    #[cfg(feature = "image-request")]
    #[error("css color parse error")]
    ReqwestError(#[from] reqwest::Error),
}

impl From<lyon_extra::parser::ParseError> for VegaSceneGraphError {
    fn from(value: lyon_extra::parser::ParseError) -> Self {
        Self::InvalidSvgPath(value)
    }
}
