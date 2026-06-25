use std::{
    collections::HashMap,
    marker::PhantomData,
    sync::{Arc, Mutex},
};

use cosmic_text::{Command, FontSystem, SwashCache, SwashContent};
use lyon_path::geom::Point;

use crate::{
    error::AvengerTextError,
    measurement::{
        cosmic::{
            make_cosmic_text_buffer, measure_font_metrics_with_cosmic, measure_text_buffer,
            FONT_SYSTEM, SWASH_CACHE,
        },
        truncate_text_to_limit_with, FontMetrics, FontMetricsConfig, TextBounds,
        TextMeasurementConfig, TextMeasurer,
    },
    rasterization::GlyphPosition,
    FontResolutionOptions,
};

use super::{
    GlyphBBox, GlyphData, TextRasterizationBuffer, TextRasterizationConfig, TextRasterizer,
};

type CosmicCacheKey = (cosmic_text::CacheKey, [u8; 4]);

#[derive(Clone)]
pub struct CosmicTextRasterizer<CacheValue> {
    resources: Arc<CosmicTextResources>,
    _phantom: PhantomData<CacheValue>,
}

impl<CacheValue> std::fmt::Debug for CosmicTextRasterizer<CacheValue> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let resources = match self.resources.as_ref() {
            CosmicTextResources::Global => "global",
            CosmicTextResources::Local { .. } => "local",
        };
        f.debug_struct("CosmicTextRasterizer")
            .field("resources", &resources)
            .finish()
    }
}

enum CosmicTextResources {
    Global,
    Local {
        font_system: Mutex<FontSystem>,
        swash_cache: Mutex<SwashCache>,
    },
}

impl<CacheValue> Default for CosmicTextRasterizer<CacheValue> {
    fn default() -> Self {
        Self::new()
    }
}

impl<CacheValue> CosmicTextRasterizer<CacheValue> {
    pub fn new() -> Self {
        Self {
            resources: Arc::new(CosmicTextResources::Global),
            _phantom: PhantomData,
        }
    }

    pub fn with_font_resolution(options: FontResolutionOptions) -> Self {
        Self {
            resources: Arc::new(CosmicTextResources::Local {
                font_system: Mutex::new(crate::fonts::build_cosmic_font_system(&options)),
                swash_cache: Mutex::new(SwashCache::new()),
            }),
            _phantom: PhantomData,
        }
    }
}

impl<CacheValue> TextMeasurer for CosmicTextRasterizer<CacheValue>
where
    CacheValue: Send + Sync,
{
    fn measure_text_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
        match self.resources.as_ref() {
            CosmicTextResources::Global => {
                let mut font_system = FONT_SYSTEM
                    .lock()
                    .expect("Failed to acquire lock on FONT_SYSTEM");
                measure_text_bounds_with_resources(config, &mut font_system)
            }
            CosmicTextResources::Local { font_system, .. } => {
                let mut font_system = font_system
                    .lock()
                    .expect("Failed to acquire local FontSystem lock");
                measure_text_bounds_with_resources(config, &mut font_system)
            }
        }
    }

    fn measure_font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics {
        match self.resources.as_ref() {
            CosmicTextResources::Global => {
                let font_system = FONT_SYSTEM
                    .lock()
                    .expect("Failed to acquire lock on FONT_SYSTEM");
                measure_font_metrics_with_cosmic(config, &font_system)
                    .unwrap_or_else(|| FontMetrics::fallback(config.font_size))
            }
            CosmicTextResources::Local { font_system, .. } => {
                let font_system = font_system
                    .lock()
                    .expect("Failed to acquire local FontSystem lock");
                measure_font_metrics_with_cosmic(config, &font_system)
                    .unwrap_or_else(|| FontMetrics::fallback(config.font_size))
            }
        }
    }
}

fn measure_text_bounds_with_resources(
    config: &TextMeasurementConfig,
    font_system: &mut FontSystem,
) -> TextBounds {
    let buffer = make_cosmic_text_buffer(config, font_system);
    measure_text_buffer(&buffer)
}

impl<CacheValue> TextRasterizer for CosmicTextRasterizer<CacheValue>
where
    CacheValue: Clone + 'static,
{
    type CacheKey = CosmicCacheKey;
    type CacheValue = CacheValue;

    fn rasterize(
        &self,
        config: &TextRasterizationConfig,
        scale: f32,
        cached_glyphs: &HashMap<Self::CacheKey, Self::CacheValue>,
    ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerTextError> {
        match self.resources.as_ref() {
            CosmicTextResources::Global => {
                let mut font_system = FONT_SYSTEM
                    .lock()
                    .expect("Failed to acquire lock on FONT_SYSTEM");
                let mut cache = SWASH_CACHE
                    .lock()
                    .expect("Failed to acquire lock on SWASH_CACHE");
                rasterize_with_resources(config, scale, cached_glyphs, &mut font_system, &mut cache)
            }
            CosmicTextResources::Local {
                font_system,
                swash_cache,
            } => {
                let mut font_system = font_system
                    .lock()
                    .expect("Failed to acquire local FontSystem lock");
                let mut cache = swash_cache
                    .lock()
                    .expect("Failed to acquire local SwashCache lock");
                rasterize_with_resources(config, scale, cached_glyphs, &mut font_system, &mut cache)
            }
        }
    }
}

fn rasterize_with_resources<CacheValue>(
    config: &TextRasterizationConfig,
    scale: f32,
    cached_glyphs: &HashMap<CosmicCacheKey, CacheValue>,
    font_system: &mut FontSystem,
    cache: &mut SwashCache,
) -> Result<TextRasterizationBuffer<CosmicCacheKey>, AvengerTextError>
where
    CacheValue: Clone,
{
    // Build image cache
    let mut next_cache: HashMap<CosmicCacheKey, GlyphData<CosmicCacheKey>> = HashMap::new();

    let text = truncate_text_to_limit_with(config.text, config.limit, |candidate| {
        let measurement_config = TextMeasurementConfig {
            text: candidate,
            font: config.font,
            font_size: config.font_size,
            font_weight: config.font_weight,
            font_style: config.font_style,
        };
        let buffer = make_cosmic_text_buffer(&measurement_config, font_system);
        measure_text_buffer(&buffer).width
    });

    let measurement_config = TextMeasurementConfig {
        text: &text,
        font: config.font,
        font_size: config.font_size,
        font_weight: config.font_weight,
        font_style: config.font_style,
    };
    let buffer = make_cosmic_text_buffer(&measurement_config, font_system);

    let text_bounds = measure_text_buffer(&buffer);

    let text_color = [
        (config.color[0] * 255.0).round() as u8,
        (config.color[1] * 255.0).round() as u8,
        (config.color[2] * 255.0).round() as u8,
        (config.color[3] * 255.0).round() as u8,
    ];

    // Initialize glyphs
    let mut glyphs: Vec<(GlyphData<CosmicCacheKey>, GlyphPosition)> = Vec::new();

    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            let physical_glyph = glyph.physical((0.0, 0.0), scale);

            let glyph_pos = GlyphPosition {
                x: glyph.x + glyph.font_size * glyph.x_offset,
                y: glyph.y - glyph.font_size * glyph.y_offset,
                physical_x: physical_glyph.x as f32,
                physical_y: physical_glyph.y as f32,
            };

            // Compute cache key which combines glyph and color
            let cache_key = (physical_glyph.cache_key, text_color);

            if let Some(glyph_image) = next_cache.get(&cache_key) {
                // Glyph has already been rasterized by this call to rasterize and the full image
                // is already in the glyphs Vec, so we can store the reference only.
                glyphs.push((glyph_image.clone().without_image(), glyph_pos));
            } else {
                // We need to rasterize glyph and write it to next_atlas
                let Some(image) = cache
                    .get_image(font_system, physical_glyph.cache_key)
                    .as_ref()
                else {
                    return Err(AvengerTextError::ImageAllocationError(
                        "Failed to create glyph image".to_string(),
                    ));
                };

                let width = image.placement.width as usize;
                let height = image.placement.height as usize;
                let should_rasterize = width > 0 && height > 0;

                if !should_rasterize {
                    continue;
                }

                let bbox = GlyphBBox {
                    top: image.placement.top,
                    left: image.placement.left,
                    width: image.placement.width,
                    height: image.placement.height,
                };

                if cached_glyphs.contains_key(&cache_key) {
                    // Glyph already rasterized by a prior call to rasterize(), so we can just
                    // store the cache key and position info.
                    glyphs.push((
                        GlyphData {
                            cache_key,
                            image: None,
                            path: None,
                            bbox,
                        },
                        glyph_pos,
                    ));
                } else {
                    let img = match image.content {
                        SwashContent::Color => {
                            // Image is rgba (like an emoji)
                            let Some(img) = image::RgbaImage::from_vec(
                                width as u32,
                                height as u32,
                                image.data.clone(),
                            ) else {
                                return Err(AvengerTextError::ImageAllocationError(
                                    "Failed to parse text rasterization as Rgba image".to_string(),
                                ));
                            };
                            img
                        }
                        SwashContent::Mask | SwashContent::SubpixelMask => {
                            // Image is monochrome (like regular text)
                            let Some(monochrome_img) = image::GrayImage::from_vec(
                                width as u32,
                                height as u32,
                                image.data.clone(),
                            ) else {
                                return Err(AvengerTextError::ImageAllocationError(
                                    "Failed to parse text rasterization as Grayscale image"
                                        .to_string(),
                                ));
                            };

                            // Initialize empty rgba image
                            let mut img = image::RgbaImage::new(
                                monochrome_img.width(),
                                monochrome_img.height(),
                            );

                            // Write colored image
                            for x in 0..monochrome_img.width() {
                                for y in 0..monochrome_img.height() {
                                    let pixel_lum = monochrome_img.get_pixel(x, y).0[0];
                                    // Compute pixel color, adjusting alpha by pixel luminance
                                    let mut pixel_color = text_color;
                                    pixel_color[3] =
                                        ((text_color[3] as f32) * (pixel_lum as f32 / 255.0))
                                            .round() as u8;

                                    // Write pixel to rgba image
                                    let pixel = image::Rgba::from(pixel_color);
                                    img.put_pixel(x, y, pixel);
                                }
                            }
                            img
                        }
                    };

                    // Get path
                    // We need to rasterize glyph and write it to next_atlas
                    let path = cache
                        .get_outline_commands(font_system, physical_glyph.cache_key)
                        .map(|commands| import_path_commands(&commands));

                    // Create new glyph image
                    let glyph_data = GlyphData {
                        cache_key: (physical_glyph.cache_key, text_color),
                        image: Some(img),
                        path,
                        bbox,
                    };

                    // Update cache
                    next_cache.insert(cache_key, glyph_data.clone().without_image());

                    glyphs.push((glyph_data, glyph_pos));
                }
            };
        }
    }

    Ok(TextRasterizationBuffer {
        glyphs,
        text_bounds,
    })
}

/// Import cosmic text path commands into a lyon path
pub(crate) fn import_path_commands(commands: &[Command]) -> lyon_path::Path {
    import_path_commands_with_offset(commands, 0.0, 0.0)
}

pub(crate) fn import_path_commands_with_offset(
    commands: &[Command],
    x_offset: f32,
    y_offset: f32,
) -> lyon_path::Path {
    let mut builder = lyon_path::Builder::new().with_svg();

    for command in commands {
        match command {
            Command::MoveTo(p) => {
                builder.move_to(Point::new(x_offset + p.x, y_offset - p.y));
            }
            Command::LineTo(p) => {
                builder.line_to(Point::new(x_offset + p.x, y_offset - p.y));
            }
            Command::CurveTo(p1, p2, p3) => {
                builder.cubic_bezier_to(
                    Point::new(x_offset + p1.x, y_offset - p1.y),
                    Point::new(x_offset + p2.x, y_offset - p2.y),
                    Point::new(x_offset + p3.x, y_offset - p3.y),
                );
            }
            Command::QuadTo(p1, p2) => {
                builder.quadratic_bezier_to(
                    Point::new(x_offset + p1.x, y_offset - p1.y),
                    Point::new(x_offset + p2.x, y_offset - p2.y),
                );
            }
            Command::Close => builder.close(),
        };
    }

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FontStyle, FontWeight, FontWeightNameSpec};

    #[test]
    fn rasterizer_applies_text_limit_before_shaping() {
        let rasterizer =
            CosmicTextRasterizer::<()>::with_font_resolution(FontResolutionOptions::default());
        let text = "Long label text".to_string();
        let color = [0.0, 0.0, 0.0, 1.0];
        let font = "Atkinson Hyperlegible Next".to_string();
        let font_weight = FontWeight::Name(FontWeightNameSpec::Normal);
        let font_style = FontStyle::Normal;
        let cached_glyphs = HashMap::new();

        let unbounded = rasterizer
            .rasterize(
                &TextRasterizationConfig {
                    text: &text,
                    color: &color,
                    font: &font,
                    font_size: 12.0,
                    font_weight: &font_weight,
                    font_style: &font_style,
                    limit: 0.0,
                },
                1.0,
                &cached_glyphs,
            )
            .unwrap();
        let limited = rasterizer
            .rasterize(
                &TextRasterizationConfig {
                    text: &text,
                    color: &color,
                    font: &font,
                    font_size: 12.0,
                    font_weight: &font_weight,
                    font_style: &font_style,
                    limit: 36.0,
                },
                1.0,
                &cached_glyphs,
            )
            .unwrap();

        assert!(unbounded.text_bounds.width > limited.text_bounds.width);
        assert!(limited.text_bounds.width <= 36.5);
    }

    #[test]
    fn rasterizer_reports_scale_independent_logical_glyph_positions() {
        let rasterizer =
            CosmicTextRasterizer::<()>::with_font_resolution(FontResolutionOptions::default());
        let text = "Rotated Label".to_string();
        let color = [0.0, 0.0, 0.0, 1.0];
        let font = "Atkinson Hyperlegible Next".to_string();
        let font_weight = FontWeight::Name(FontWeightNameSpec::Normal);
        let font_style = FontStyle::Normal;
        let cached_glyphs = HashMap::new();
        let config = TextRasterizationConfig {
            text: &text,
            color: &color,
            font: &font,
            font_size: 12.0,
            font_weight: &font_weight,
            font_style: &font_style,
            limit: 0.0,
        };

        let scale_one = rasterizer.rasterize(&config, 1.0, &cached_glyphs).unwrap();
        let scale_two = rasterizer.rasterize(&config, 2.0, &cached_glyphs).unwrap();

        assert_eq!(scale_one.glyphs.len(), scale_two.glyphs.len());
        for ((_, pos_one), (_, pos_two)) in scale_one.glyphs.iter().zip(&scale_two.glyphs) {
            assert!((pos_one.x - pos_two.x).abs() < 0.001);
            assert!((pos_one.y - pos_two.y).abs() < 0.001);
        }

        let has_scaled_physical_position =
            scale_one
                .glyphs
                .iter()
                .zip(&scale_two.glyphs)
                .any(|((_, pos_one), (_, pos_two))| {
                    (pos_one.physical_x - pos_two.physical_x).abs() > 0.001
                        || (pos_one.physical_y - pos_two.physical_y).abs() > 0.001
                });
        assert!(has_scaled_physical_position);
    }
}
