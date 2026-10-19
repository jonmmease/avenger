use thiserror::Error;

#[derive(Debug, Error)]
pub enum AvengerSvgError {
    #[error("invalid geometry: {0}")]
    InvalidGeometry(String),
    #[error("invalid pattern: {0}")]
    InvalidPattern(#[from] avenger_scenegraph::marks::pattern::PatternValidationError),
    #[error("invalid paint: {0}")]
    InvalidPaint(#[from] avenger_scenegraph::render::paint::GradientIndexError),
    #[error("image encoding error: {0}")]
    ImageEncoding(String),
    #[error("image resources aren't ready: {0:?}")]
    ImagesNotReady(Vec<String>),
    #[error("text error: {0}")]
    Text(#[from] avenger_typst_label::LabelError),
    #[error("font error: {0}")]
    Font(String),
}
