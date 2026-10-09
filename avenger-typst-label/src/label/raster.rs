//! Labels rasterized whole, placed in their boxes.

use std::sync::Arc;

use image::RgbaImage;

use super::bounds::{TextBounds, first_baseline};
use super::engine::CompiledLabel;
use super::memo::TextRasterKey;
use crate::typst_render::{RasterError, RasterOptions, rasterize};

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

/// A compiled label's raster at a scale, with its box.
pub(crate) fn raster(
    label: &CompiledLabel,
    font_size: f32,
    scale: f32,
    key: TextRasterKey,
) -> Result<TextRaster, RasterError> {
    let bounds = TextBounds::new(&label.metrics, font_size);
    if label.source.is_empty() {
        return Ok(TextRaster { key, bounds, image: None, x: 0.0, y: 0.0 });
    }
    let raster = rasterize(label, &RasterOptions { scale })?;
    let image =
        RgbaImage::from_vec(raster.image.width, raster.image.height, raster.image.data)
            .expect("a label raster's data matches its size");
    Ok(TextRaster {
        key,
        bounds,
        image: Some(Arc::new(image)),
        x: raster.origin_x,
        y: raster.origin_y - first_baseline(&label.metrics),
    })
}
