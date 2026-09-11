use std::hash::Hash;

use ordered_float::OrderedFloat;

use crate::{
    measurement::TextBounds,
    types::{FontStyle, FontWeight, TextSyntaxMode},
};

/// Rasterized text-line origin in text layout coordinates.
///
/// `x` and `y` are floating-point logical positions. `physical_x` and
/// `physical_y` are the pixel-aligned physical positions for the rasterized
/// bitmap at the requested scale. Renderers can use logical positions for
/// transformed/vector text and physical positions for crisp, untransformed
/// bitmap placement.
#[derive(Debug, Clone)]
pub struct TextRasterPosition {
    pub x: f32,
    pub y: f32,
    pub physical_x: f32,
    pub physical_y: f32,
}

/// Text raster bounding box relative to the text raster origin.
#[derive(Clone, Copy, Debug)]
pub struct TextRasterBBox {
    pub top: i32,
    pub left: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct TextRasterizationConfig<'a> {
    pub text: &'a str,
    pub color: [f32; 4],
    pub font: &'a str,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    /// Positive finite width in logical pixels. Plain text uses grapheme-safe
    /// ellipsis; Typst markup is compiled intact and clipped at this width.
    /// Other values leave the label unconstrained.
    pub limit: f32,
    pub syntax_mode: TextSyntaxMode,
    pub params: &'a avenger_typst_label::LabelParams,
    pub number_locale: Option<&'a str>,
    pub number_locale_specs: Option<&'a crate::NumberLocaleSpecs>,
    pub datetime_locale: Option<&'a str>,
    pub datetime_timezone: Option<&'a str>,
    pub datetime_locale_specs: Option<&'a crate::DateTimeLocaleSpecs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextRasterCacheKey {
    pub limit: OrderedFloat<f32>,
    pub text: String,
    pub font: String,
    pub font_size: OrderedFloat<f32>,
    pub font_weight: String,
    pub font_style: String,
    pub fill: [u8; 4],
    pub scale: OrderedFloat<f32>,
    pub markup: String,
    pub params: String,
    pub number_locale: Option<String>,
    pub number_locale_specs: String,
    pub datetime_locale: Option<String>,
    pub datetime_timezone: Option<String>,
    pub datetime_locale_specs: String,
}

#[derive(Clone)]
pub struct TextRasterEntry<CacheKey: Hash + Eq + Clone> {
    pub cache_key: CacheKey,
    /// None if an image for the same cache key was already included.
    pub image: Option<image::RgbaImage>,
    pub bbox: TextRasterBBox,
}

#[derive(Clone)]
pub struct TextRasterizationBuffer<CacheKey: Hash + Eq + Clone> {
    pub entries: Vec<(TextRasterEntry<CacheKey>, TextRasterPosition)>,
    pub text_bounds: TextBounds,
}

#[derive(Clone)]
pub struct CachedTextRasterization {
    pub entries: Vec<(TextRasterEntry<TextRasterCacheKey>, TextRasterPosition)>,
    pub text_bounds: TextBounds,
}

impl CachedTextRasterization {
    pub fn as_buffer(&self) -> TextRasterizationBuffer<TextRasterCacheKey> {
        TextRasterizationBuffer {
            entries: self.entries.clone(),
            text_bounds: self.text_bounds.clone(),
        }
    }
}

pub trait TextRasterCacheValue: Clone {
    fn cached_text_rasterization(&self) -> Option<TextRasterizationBuffer<TextRasterCacheKey>>;
}

impl TextRasterCacheValue for () {
    fn cached_text_rasterization(&self) -> Option<TextRasterizationBuffer<TextRasterCacheKey>> {
        None
    }
}

impl TextRasterCacheValue for CachedTextRasterization {
    fn cached_text_rasterization(&self) -> Option<TextRasterizationBuffer<TextRasterCacheKey>> {
        Some(self.as_buffer())
    }
}
