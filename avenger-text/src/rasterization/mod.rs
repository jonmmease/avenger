use std::{collections::HashMap, hash::Hash};

#[cfg(feature = "cosmic-text")]
pub mod cosmic;

#[cfg(target_arch = "wasm32")]
pub mod html_canvas;

use crate::{
    error::AvengerTextError,
    measurement::{TextBounds, TextMeasurementConfig},
    types::{FontStyle, FontWeight},
};

// Position of glyph in text buffer
#[derive(Debug, Clone)]
pub struct PhysicalGlyphPosition {
    pub x: f32,
    pub y: f32,
}

// Glyph bounding box relative to glyph origin
#[derive(Clone, Copy, Debug)]
pub struct GlyphBBox {
    pub top: i32,
    pub left: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub struct GlyphImage<CacheKey: Hash + Eq + Clone> {
    pub cache_key: CacheKey,
    // None if image for same CacheKey was already included
    pub image: Option<image::RgbaImage>,
    pub bbox: GlyphBBox,
}

impl<CacheKey: Hash + Eq + Clone> GlyphImage<CacheKey> {
    pub fn without_image(&self) -> Self {
        Self {
            cache_key: self.cache_key.clone(),
            image: None,
            bbox: self.bbox,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextRasterizationConfig<'a> {
    pub text: &'a String,
    pub color: &'a [f32; 4],
    pub font: &'a String,
    pub font_size: f32,
    pub font_weight: &'a FontWeight,
    pub font_style: &'a FontStyle,
    pub limit: f32,
}

impl<'a> TextRasterizationConfig<'a> {
    pub fn to_measurement_config(&self) -> TextMeasurementConfig<'a> {
        TextMeasurementConfig {
            text: self.text,
            font: self.font,
            font_size: self.font_size,
            font_weight: self.font_weight,
            font_style: self.font_style,
        }
    }
}

#[derive(Clone)]
pub struct GlyphData<CacheKey: Hash + Eq + Clone> {
    pub cache_key: CacheKey,
    // image and path are None if the CacheKey was already included
    pub image: Option<image::RgbaImage>,
    pub path: Option<lyon_path::Path>,
    pub bbox: GlyphBBox,
}

impl<CacheKey: Hash + Eq + Clone> GlyphData<CacheKey> {
    pub fn without_image(self) -> Self {
        Self {
            image: None,
            ..self
        }
    }

    pub fn with_bbox(self, bbox: GlyphBBox) -> Self {
        Self {
            bbox,
            ..self.clone()
        }
    }
}

#[derive(Clone)]
pub struct TextRasterizationBuffer<CacheKey: Hash + Eq + Clone> {
    pub glyphs: Vec<(GlyphData<CacheKey>, PhysicalGlyphPosition)>,
    pub text_bounds: TextBounds,
}

pub trait TextRasterizer: 'static {
    type CacheKey: Hash + Eq + Clone;
    type CacheValue: Clone;

    fn rasterize(
        &self,
        config: &TextRasterizationConfig,
        scale: f32,
        cached_glyphs: &HashMap<Self::CacheKey, Self::CacheValue>,
    ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerTextError>;
}

#[cfg(feature = "cosmic-text")]
pub fn default_rasterizer() -> impl TextRasterizer<CacheValue = ()> {
    crate::rasterization::cosmic::CosmicTextRasterizer::new()
}

#[cfg(target_arch = "wasm32")]
pub fn default_rasterizer() -> impl TextRasterizer<CacheValue = ()> {
    return crate::rasterization::html_canvas::HtmlCanvasTextRasterizer::new();
}
