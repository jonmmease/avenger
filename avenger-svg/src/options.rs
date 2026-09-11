use avenger_text::FontResolutionOptions;

#[derive(Debug, Clone, PartialEq)]
/// SVG appearance and font configuration.
pub struct SvgRenderOptions {
    pub background: SvgBackground,
    /// Number of fractional digits in geometry coordinates.
    pub precision: usize,
    /// Used when the renderer does not have a caller-supplied text engine.
    pub font_resolution: FontResolutionOptions,
    pub font_embedding: SvgFontEmbedding,
    /// Embed color glyph images supplied by the text engine. Enabled by default.
    pub rasterize_color_emoji: bool,
}

impl Default for SvgRenderOptions {
    fn default() -> Self {
        Self {
            background: SvgBackground::White,
            precision: 3,
            font_resolution: avenger_text::default_font_resolution(),
            font_embedding: SvgFontEmbedding::EmbedSubsetWoff2,
            rasterize_color_emoji: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
/// Background paint for the document viewport.
pub enum SvgBackground {
    White,
    Transparent,
    Color([f32; 4]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Font resources included in the SVG document.
pub enum SvgFontEmbedding {
    /// Embed resolved faces. Preserve complete fonts when subsetting loses shaping tables.
    EmbedSubsetWoff2,
    /// Require the viewer to supply the resolved fonts. Layout still requires local fonts.
    None,
}
