use thiserror::Error;

#[derive(Debug, Error)]
pub enum AvengerSvgError {
    #[error("invalid geometry: {0}")]
    InvalidGeometry(String),
    #[error("unsupported mark type: {0}")]
    UnsupportedMark(&'static str),
    #[error("unsupported paint: {0}")]
    UnsupportedPaint(String),
    #[error("unsupported feature: {0}")]
    UnsupportedFeature(String),
    #[error("image encoding error: {0}")]
    ImageEncoding(String),
    #[error("text error: {0}")]
    Text(String),
    #[error("font error: {0}")]
    Font(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
