use std::{collections::HashMap, hash::Hash};

use avenger_typst_label::LabelEngine;
use ordered_float::OrderedFloat;

use crate::{
    error::AvengerTextError,
    measurement::TextBounds,
    types::{TextConfig, TextSyntaxMode},
    typeset::{
        bounds_from_metrics, first_baseline, label_params_fingerprint, typeset, LabelSettings,
    },
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextRasterCacheKey {
    pub text: String,
    pub syntax_mode: TextSyntaxMode,
    /// The layout, in its debug form, which tells apart every distance.
    pub layout: String,
    pub font: String,
    pub font_size: OrderedFloat<f32>,
    pub font_weight: String,
    pub font_style: String,
    pub fill: [u8; 4],
    pub scale: OrderedFloat<f32>,
    pub markup: String,
    pub params: String,
    pub number_format: Option<crate::ProviderIdentity<dyn crate::NumberFormatProvider>>,
    pub datetime_format: Option<crate::ProviderIdentity<dyn crate::DateTimeFormatProvider>>,
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
            text_bounds: self.text_bounds,
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

/// Rasterizes a label at a scale, unless a cached entry has its key.
pub(crate) fn rasterize<CacheValue: TextRasterCacheValue>(
    typst: &LabelEngine,
    settings: &LabelSettings,
    config: &TextConfig,
    scale: f32,
    cached_entries: &HashMap<TextRasterCacheKey, CacheValue>,
) -> Result<TextRasterizationBuffer<TextRasterCacheKey>, AvengerTextError> {
    let cache_key = TextRasterCacheKey {
        text: config.text.to_string(),
        syntax_mode: config.syntax_mode,
        layout: format!("{:?}", config.layout),
        font: config.font.to_string(),
        font_size: OrderedFloat(config.font_size),
        font_weight: format!("{:?}", config.font_weight),
        font_style: format!("{:?}", config.font_style),
        fill: color_key(&config.color),
        scale: OrderedFloat(scale),
        markup: format!("{settings:?}"),
        params: label_params_fingerprint(config.params),
        number_format: config
            .number_format
            .or(typst.number_format())
            .map(crate::ProviderIdentity::new),
        datetime_format: config
            .datetime_format
            .or(typst.datetime_format())
            .map(crate::ProviderIdentity::new),
    };
    if let Some(cached) = cached_entries
        .get(&cache_key)
        .and_then(TextRasterCacheValue::cached_text_rasterization)
    {
        return Ok(cached);
    }

    let label = typeset(typst, settings, config)?;
    let bounds = bounds_from_metrics(&label.metrics, config.font_size);
    if config.text.is_empty() {
        return Ok(TextRasterizationBuffer {
            text_bounds: bounds,
            entries: Vec::new(),
        });
    }
    let baseline = first_baseline(&label.metrics);
    let raster =
        avenger_typst_label::rasterize(&label, &avenger_typst_label::RasterOptions { scale })?;
    let image = if cached_entries.contains_key(&cache_key) {
        None
    } else {
        Some(
            image::RgbaImage::from_vec(raster.image.width, raster.image.height, raster.image.data)
                .ok_or_else(|| {
                    AvengerTextError::ImageAllocationError(
                        "Typst text raster image dimensions did not match data".to_string(),
                    )
                })?,
        )
    };

    Ok(TextRasterizationBuffer {
        text_bounds: bounds,
        entries: vec![(
            TextRasterEntry {
                cache_key,
                image,
                bbox: TextRasterBBox {
                    top: 0,
                    left: 0,
                    width: raster.image.width,
                    height: raster.image.height,
                },
            },
            TextRasterPosition {
                x: raster.origin_x,
                y: raster.origin_y - baseline,
                physical_x: (raster.origin_x * scale).round(),
                physical_y: ((raster.origin_y - baseline) * scale).round(),
            },
        )],
    })
}

fn color_key(color: &[f32; 4]) -> [u8; 4] {
    [
        channel(color[0]),
        channel(color[1]),
        channel(color[2]),
        channel(color[3]),
    ]
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
