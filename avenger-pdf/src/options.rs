#[derive(Debug, Clone, PartialEq)]
/// PDF appearance and stream compression.
pub struct PdfRenderOptions {
    /// Background fill for the generated PDF page.
    pub background: PdfBackground,
    /// Compress PDF streams where supported by the writer.
    pub compress: bool,
}

impl Default for PdfRenderOptions {
    fn default() -> Self {
        Self {
            background: PdfBackground::White,
            compress: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
/// Background paint for the page.
pub enum PdfBackground {
    /// Fill the page with white before drawing scene marks.
    White,
    /// Leave the page background transparent.
    Transparent,
    /// Fill the page with an explicit RGBA color.
    Color([f32; 4]),
}
