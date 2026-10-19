use std::string::FromUtf8Error;

#[derive(Debug, thiserror::Error)]
pub enum AvengerImageError {
    #[error("Internal error: {0}")]
    InternalError(String),

    #[error("Image fetching is not enabled: {0}")]
    NoImageFetcherConfigured(String),

    #[error("HTTP status {status} for {url}")]
    HttpStatus { url: String, status: u16 },

    #[error("SVG image support is not enabled: {0}")]
    SvgSupportDisabled(String),

    #[error("image error")]
    ImageError(#[from] image::ImageError),

    #[error("base64 decode error")]
    Base64DecodeError(#[from] base64::DecodeError),

    #[error("string decode error")]
    FromUtf8Error(#[from] FromUtf8Error),

    #[cfg(feature = "reqwest")]
    #[error("HTTP request failed: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[cfg(feature = "svg")]
    #[error("usvg error: {0}")]
    UsvgError(#[from] usvg::Error),

    #[cfg(feature = "svg")]
    #[error("roxml Error: {0}")]
    RoxmlError(#[from] usvg::roxmltree::Error),
}
