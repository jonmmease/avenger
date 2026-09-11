use avenger_text::FontResolutionOptions;

#[derive(Debug, Clone, PartialEq)]
pub struct SvgRenderOptions {
    pub background: SvgBackground,
    pub precision: usize,
    pub image_mode: SvgImageMode,
    pub font_resolution: FontResolutionOptions,
    pub font_embedding: SvgFontEmbedding,
    pub include_metadata: bool,
    pub rasterize_color_emoji: bool,
}

impl Default for SvgRenderOptions {
    fn default() -> Self {
        Self {
            background: SvgBackground::White,
            precision: 3,
            image_mode: SvgImageMode::EmbedPngDataUris,
            font_resolution: FontResolutionOptions::default(),
            font_embedding: SvgFontEmbedding::EmbedSubsetWoff2,
            include_metadata: false,
            rasterize_color_emoji: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SvgBackground {
    White,
    Transparent,
    Color([f32; 4]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgImageMode {
    EmbedPngDataUris,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgFontEmbedding {
    EmbedSubsetWoff2,
    None,
}
