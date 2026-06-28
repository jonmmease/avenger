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
    pub limit: f32,
    pub syntax_mode: TextSyntaxMode,
    pub params: &'a avenger_typst_label::LabelParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextRasterCacheKey {
    pub text: String,
    pub font: String,
    pub font_size: OrderedFloat<f32>,
    pub font_weight: String,
    pub font_style: String,
    pub fill: [u8; 4],
    pub scale: OrderedFloat<f32>,
    pub markup: String,
    pub params: String,
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
