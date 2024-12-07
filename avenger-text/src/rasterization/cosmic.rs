use std::{collections::HashMap, marker::PhantomData};

use avenger_common::canvas::CanvasDimensions;
use cosmic_text::{Command, SwashContent};
use lyon_path::geom::Point;

use crate::{
    error::AvengerTextError,
    measurement::cosmic::{make_cosmic_text_buffer, measure_text_buffer},
    rasterization::PhysicalGlyphPosition,
};

use super::{
    GlyphBBox, GlyphData, TextRasterizationBuffer, TextRasterizationConfig, TextRasterizer,
};

use crate::measurement::cosmic::{FONT_SYSTEM, SWASH_CACHE};

type CosmicCacheKey = (cosmic_text::CacheKey, [u8; 4]);

#[derive(Clone, Debug)]
pub struct CosmicTextRasterizer<CacheValue> {
    _phantom: PhantomData<CacheValue>,
}

impl<CacheValue> CosmicTextRasterizer<CacheValue> {
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<CacheValue> TextRasterizer for CosmicTextRasterizer<CacheValue>
where
    CacheValue: Clone + 'static,
{
    type CacheKey = CosmicCacheKey;
    type CacheValue = CacheValue;

    fn rasterize(
        &self,
        dimensions: &CanvasDimensions,
        config: &TextRasterizationConfig,
        cached_glyphs: &HashMap<Self::CacheKey, Self::CacheValue>,
    ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerTextError> {
        let mut font_system = FONT_SYSTEM
            .lock()
            .expect("Failed to acquire lock on FONT_SYSTEM");

        let mut cache = SWASH_CACHE
            .lock()
            .expect("Failed to acquire lock on SWASH_CACHE");

        // Build image cache
        let mut next_cache: HashMap<CosmicCacheKey, GlyphData<CosmicCacheKey>> = HashMap::new();

        let buffer = make_cosmic_text_buffer(
            &config.to_measurement_config(),
            dimensions,
            &mut font_system,
        );

        let text_bounds = measure_text_buffer(&buffer);

        let text_color = [
            (config.color[0] * 255.0).round() as u8,
            (config.color[1] * 255.0).round() as u8,
            (config.color[2] * 255.0).round() as u8,
            (config.color[3] * 255.0).round() as u8,
        ];

        // Initialize glyphs
        let mut glyphs: Vec<GlyphData<CosmicCacheKey>> = Vec::new();

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical_glyph = glyph.physical((0.0, 0.0), dimensions.scale);

                let phys_pos = PhysicalGlyphPosition {
                    x: physical_glyph.x as f32,
                    y: physical_glyph.y as f32,
                };

                // Compute cache key which combines glyph and color
                let cache_key = (physical_glyph.cache_key, text_color);

                if let Some(glyph_image) = next_cache.get(&cache_key) {
                    // Glyph has already been rasterized by this call to rasterize and the full image
                    // is already in the glyphs Vec, so we can store the reference only.
                    glyphs.push(
                        glyph_image
                            .clone()
                            .without_image_and_path()
                            .with_physical_position(phys_pos),
                    );
                } else {
                    // We need to rasterize glyph and write it to next_atlas
                    let Some(image) = cache
                        .get_image(&mut font_system, physical_glyph.cache_key)
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
                        glyphs.push(GlyphData {
                            cache_key,
                            image: None,
                            path: None,
                            bbox,
                            physical_position: phys_pos,
                        });
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
                                        "Failed to parse text rasterization as Rgba image"
                                            .to_string(),
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
                                        pixel_color[3] = ((text_color[3] as f32)
                                            * (pixel_lum as f32 / 255.0))
                                            .round()
                                            as u8;

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
                        let path = if let Some(outline_commands) =
                            cache.get_outline_commands(&mut font_system, physical_glyph.cache_key)
                        {
                            Some(import_path_commands(&outline_commands))
                        } else {
                            // Path data not available, leave as None
                            None
                        };

                        // Create new glyph image
                        let glyph_data = GlyphData {
                            cache_key: (physical_glyph.cache_key, text_color),
                            image: Some(img),
                            path,
                            bbox,
                            physical_position: phys_pos,
                        };

                        // Update cache
                        next_cache.insert(cache_key, glyph_data.clone().without_image_and_path());

                        glyphs.push(glyph_data);
                    }
                };
            }
        }

        Ok(TextRasterizationBuffer {
            glyphs,
            text_bounds,
        })
    }
}

/// Import cosmic text path commands into a lyon path
fn import_path_commands(commands: &[Command]) -> lyon_path::Path {
    let mut builder = lyon_path::Builder::new().with_svg();

    for command in commands {
        match command {
            Command::MoveTo(p) => {
                builder.move_to(Point::new(p.x, -p.y));
            }
            Command::LineTo(p) => {
                builder.line_to(Point::new(p.x, -p.y));
            }
            Command::CurveTo(p1, p2, p3) => {
                builder.cubic_bezier_to(
                    Point::new(p1.x, -p1.y),
                    Point::new(p2.x, -p2.y),
                    Point::new(p3.x, -p3.y),
                );
            }
            Command::QuadTo(p1, p2) => {
                builder.quadratic_bezier_to(Point::new(p1.x, -p1.y), Point::new(p2.x, -p2.y));
            }
            Command::Close => builder.close(),
        };
    }

    builder.build()
}
