use thiserror::Error;

#[derive(Debug, Error)]
pub enum AvengerPdfError {
    #[error("invalid PDF page size {width}x{height}")]
    InvalidPageSize { width: f32, height: f32 },
    #[error("invalid geometry: {0}")]
    InvalidGeometry(String),
    #[error("invalid pattern: {0}")]
    InvalidPattern(#[from] avenger_scenegraph::marks::pattern::PatternValidationError),
    #[error("invalid paint: {0}")]
    InvalidPaint(#[from] avenger_scenegraph::render::paint::GradientIndexError),
    #[error("image embedding error: {0}")]
    Image(String),
    #[error("text error: {0}")]
    Text(#[from] avenger_typst_label::LabelError),
    #[error("font error: {0}")]
    Font(String),
    #[error("PDF writer error: {0}")]
    Writer(#[from] krilla::error::KrillaError),
}
