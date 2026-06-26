use avenger_text::FontResolutionOptions;

#[derive(Debug, Clone, PartialEq)]
pub struct PdfRenderOptions {
    /// Background fill for the generated PDF page.
    pub background: PdfBackground,
    /// Decimal precision retained for API parity with `avenger-pdf`.
    pub precision: usize,
    /// Font sources used for selectable PDF text.
    ///
    /// Avenger embeds its bundled fonts by default. When `load_system_fonts` or
    /// `extra_font_dirs` are enabled, callers are responsible for ensuring the
    /// selected font licenses permit PDF embedding.
    pub font_resolution: FontResolutionOptions,
    /// Compress PDF streams where supported by the writer.
    pub compress: bool,
    /// Retained for API parity with `avenger-pdf`; unused by the direct path.
    pub raster_scale: f32,
}

impl Default for PdfRenderOptions {
    fn default() -> Self {
        Self {
            background: PdfBackground::White,
            precision: 3,
            font_resolution: FontResolutionOptions::default(),
            compress: true,
            raster_scale: 1.5,
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
