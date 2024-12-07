use std::{collections::HashMap, hash::Hash};

use avenger_common::canvas::CanvasDimensions;

#[cfg(feature = "cosmic-text")]
pub mod cosmic;

#[cfg(target_arch = "wasm32")]
pub mod html_canvas;

use crate::{
    error::AvengerTextError,
    measurement::{TextBounds, TextMeasurementConfig},
    types::{FontStyleSpec, FontWeightSpec},
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
    pub font_weight: &'a FontWeightSpec,
    pub font_style: &'a FontStyleSpec,
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
    pub physical_position: PhysicalGlyphPosition,
}

impl<CacheKey: Hash + Eq + Clone> GlyphData<CacheKey> {
    pub fn without_image_and_path(self) -> Self {
        Self {
            cache_key: self.cache_key,
            image: None,
            path: None,
            bbox: self.bbox,
            physical_position: self.physical_position,
        }
    }

    pub fn with_physical_position(self, physical_position: PhysicalGlyphPosition) -> Self {
        Self {
            physical_position,
            ..self.clone()
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
    pub glyphs: Vec<GlyphData<CacheKey>>,
    pub text_bounds: TextBounds,
}

pub trait TextRasterizer: 'static {
    type CacheKey: Hash + Eq + Clone;
    type CacheValue: Clone;

    fn rasterize(
        &self,
        dimensions: &CanvasDimensions,
        config: &TextRasterizationConfig,
        cached_glyphs: &HashMap<Self::CacheKey, Self::CacheValue>,
    ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerTextError>;
}
