use thiserror::Error;

#[derive(Debug, Error)]
pub enum AvengerPdfError {
    #[error("invalid PDF page size {width}x{height}")]
    InvalidPageSize { width: f32, height: f32 },
    #[error("PDF render error: {0}")]
    Render(#[from] krilla::error::KrillaError),
    #[error("font embedding error: {0}")]
    Font(String),
    #[error("PDF conversion error: {0}")]
    Conversion(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
