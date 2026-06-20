use thiserror::Error;

#[derive(Debug, Error)]
pub enum AvengerPdfError {
    #[error("SVG render error: {0}")]
    Svg(#[from] avenger_svg::AvengerSvgError),
    #[error("SVG parse error: {0}")]
    SvgParse(#[from] svg2pdf::usvg::Error),
    #[error("font embedding error: {0}")]
    Font(String),
    #[error("PDF conversion error: {0}")]
    Conversion(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
