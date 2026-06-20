use avenger_text::FontResolutionOptions;

#[derive(Debug, Clone, PartialEq)]
pub struct PdfRenderOptions {
    pub background: PdfBackground,
    pub precision: usize,
    pub font_resolution: FontResolutionOptions,
    pub compress: bool,
    pub raster_scale: f32,
    pub embed_text: bool,
}

impl Default for PdfRenderOptions {
    fn default() -> Self {
        Self {
            background: PdfBackground::White,
            precision: 3,
            font_resolution: FontResolutionOptions::default(),
            compress: true,
            raster_scale: 1.5,
            embed_text: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PdfBackground {
    White,
    Transparent,
    Color([f32; 4]),
}
