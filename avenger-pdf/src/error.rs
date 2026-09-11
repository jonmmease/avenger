use thiserror::Error;

#[derive(Debug, Error)]
pub enum AvengerPdfError {
    #[error("invalid PDF page size {width}x{height}")]
    InvalidPageSize { width: f32, height: f32 },
    #[error("invalid geometry: {0}")]
    InvalidGeometry(String),
    #[error("PDF render error: {0}")]
    Render(#[from] krilla::error::KrillaError),
    #[error("font embedding error: {0}")]
    Font(String),
    #[error("unsupported PDF feature: {0}")]
    UnsupportedFeature(String),
    #[error("image embedding error: {0}")]
    Image(String),
    #[error("text rendering error: {0}")]
    Text(#[from] avenger_text::error::AvengerTextError),
    #[error("invalid PDF text buffer: {0}")]
    TextBuffer(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
