use avenger_text::FontResolutionOptions;

#[derive(Debug, Clone, PartialEq)]
/// PDF appearance, font configuration, and stream compression.
pub struct PdfRenderOptions {
    /// Background fill for the generated PDF page.
    pub background: PdfBackground,
    /// Font sources used when no caller-supplied text engine is set.
    /// Defaults include bundled fonts and system font discovery.
    pub font_resolution: FontResolutionOptions,
    /// Compress PDF streams where supported by the writer.
    pub compress: bool,
}

impl Default for PdfRenderOptions {
    fn default() -> Self {
        Self {
            background: PdfBackground::White,
            font_resolution: avenger_text::default_font_resolution(),
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
