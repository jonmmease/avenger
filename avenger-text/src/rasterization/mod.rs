use std::sync::Arc;

use avenger_typst_label::{LabelEngine, RasterOptions};
use image::RgbaImage;

use crate::{
    cache::LabelKey,
    error::AvengerTextError,
    measurement::TextBounds,
    types::TextConfig,
    typeset::{bounds_from_metrics, first_baseline, typeset, LabelSettings},
};

/// A label rasterized at a scale.
#[derive(Debug, Clone)]
pub struct TextRaster {
    /// Rasters with equal keys are alike.
    pub key: TextRasterKey,
    /// The label's box.
    pub bounds: TextBounds,
    /// The image, at the scale, or none for an empty label.
    pub image: Option<Arc<RgbaImage>>,
    /// The image's left edge, from the left of the box.
    pub x: f32,
    /// The image's top, from the first line's baseline.
    pub y: f32,
}

/// What a label's raster depends on: what sets its layout, its fill and the scale.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextRasterKey {
    label: LabelKey,
    fill: [u32; 4],
    scale: u32,
}

impl TextRasterKey {
    pub(crate) fn new(label: LabelKey, config: &TextConfig, scale: f32) -> Self {
        Self {
            label,
            fill: config.color.map(f32::to_bits),
            scale: scale.to_bits(),
        }
    }
}

/// Rasterizes a label at a scale.
pub(crate) fn rasterize(
    typst: &LabelEngine,
    settings: &LabelSettings,
    config: &TextConfig,
    scale: f32,
    key: TextRasterKey,
) -> Result<TextRaster, AvengerTextError> {
    let label = typeset(typst, settings, config)?;
    let bounds = bounds_from_metrics(&label.metrics, config.font_size);
    if config.text.is_empty() {
        return Ok(TextRaster {
            key,
            bounds,
            image: None,
            x: 0.0,
            y: 0.0,
        });
    }
    let raster = avenger_typst_label::rasterize(&label, &RasterOptions { scale })?;
    let image = RgbaImage::from_vec(raster.image.width, raster.image.height, raster.image.data)
        .expect("a label raster's data matches its size");
    Ok(TextRaster {
        key,
        bounds,
        image: Some(Arc::new(image)),
        x: raster.origin_x,
        y: raster.origin_y - first_baseline(&label.metrics),
    })
}
