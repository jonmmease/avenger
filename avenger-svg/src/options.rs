/// SVG output options.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgRenderOptions {
    /// The document's background color, or `None` for a transparent document. White by default.
    pub background: Option<[f32; 4]>,
    /// The number of fractional digits in coordinates.
    pub precision: usize,
    /// Embed the font faces that text uses, subset to its characters when subsetting keeps the
    /// faces' shaping tables and their licenses allow it. A face whose license forbids embedding
    /// isn't embedded. Viewers draw text without an embedded face with the fonts they have.
    /// Enabled by default.
    pub embed_fonts: bool,
    /// Draw color glyphs, such as emoji, as the images that the text engine supplies, instead of
    /// as text. Enabled by default.
    pub color_glyph_images: bool,
}

impl Default for SvgRenderOptions {
    fn default() -> Self {
        Self {
            background: Some([1.0; 4]),
            precision: 3,
            embed_fonts: true,
            color_glyph_images: true,
        }
    }
}
