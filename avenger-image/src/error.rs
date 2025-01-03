#[derive(Debug, thiserror::Error)]
pub enum AvengerImageError {
    #[error("Internal error: {0}")]
    InternalError(String),

    #[error("Image fetching is not enabled: {0}")]
    NoImageFetcherConfigured(String),

    #[error("SVG image support is not enabled: {0}")]
    SvgSupportDisabled(String),

    #[error("image error")]
    ImageError(#[from] image::ImageError),

    #[cfg(feature = "image-request")]
    #[error("css color parse error")]
    ReqwestError(#[from] reqwest::Error),

    #[cfg(feature = "svg")]
    #[error("usvg error: {0}")]
    UsvgError(#[from] usvg::Error),

    #[cfg(feature = "svg")]
    #[error("roxml Error: {0}")]
    RoxmlError(#[from] usvg::roxmltree::Error),
}
