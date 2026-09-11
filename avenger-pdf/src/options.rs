use avenger_text::FontResolutionOptions;

#[derive(Debug, Clone, PartialEq)]
pub struct PdfRenderOptions {
    /// Background fill for the generated PDF page.
    pub background: PdfBackground,
    /// Font sources used for selectable PDF text.
    ///
    /// Avenger embeds its bundled fonts by default. When `load_system_fonts` or
    /// `extra_font_dirs` are enabled, callers are responsible for ensuring the
    /// selected font licenses permit PDF embedding.
    pub font_resolution: FontResolutionOptions,
    /// Compress PDF streams where supported by the writer.
    pub compress: bool,
}

impl Default for PdfRenderOptions {
    fn default() -> Self {
        Self {
            background: PdfBackground::White,
            font_resolution: FontResolutionOptions::default(),
            compress: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PdfBackground {
    /// Fill the page with white before drawing scene marks.
    White,
    /// Leave the page background transparent.
    Transparent,
    /// Fill the page with an explicit RGBA color.
    Color([f32; 4]),
}
