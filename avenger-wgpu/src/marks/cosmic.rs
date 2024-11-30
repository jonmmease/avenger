use crate::canvas::CanvasDimensions;
use crate::error::AvengerWgpuError;
use crate::marks::text::{
    GlyphBBox, GlyphBBoxAndAtlasCoords, GlyphImage, PhysicalGlyphPosition, TextRasterizationBuffer,
    TextRasterizationConfig, TextRasterizer,
};
use avenger::marks::text::{FontWeightNameSpec, FontWeightSpec};
use cosmic_text::fontdb::Database;
use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight,
};
use lazy_static;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

lazy_static! {
    static ref FONT_SYSTEM: Mutex<FontSystem> = Mutex::new(build_font_system());
    static ref SWASH_CACHE: Mutex<SwashCache> = Mutex::new(SwashCache::new());
}

fn build_font_system() -> FontSystem {
    let mut font_system = FontSystem::new();

    // Override default families based on what system fonts are available
    let fontdb = font_system.db_mut();
    setup_default_fonts(fontdb);
    font_system
}

fn setup_default_fonts(fontdb: &mut Database) {
    let families: HashSet<String> = fontdb
        .faces()
        .flat_map(|face| {
            face.families
                .iter()
                .map(|(fam, _lang)| fam.clone())
                .collect::<Vec<_>>()
        })
        .collect();

    // Set default sans serif
    for family in ["Helvetica", "Arial", "Liberation Sans"] {
        if families.contains(family) {
            fontdb.set_sans_serif_family(family);
            break;
        }
    }

    // Set default monospace font family
    for family in [
        "Courier New",
        "Courier",
        "Liberation Mono",
        "DejaVu Sans Mono",
    ] {
        if families.contains(family) {
            fontdb.set_monospace_family(family);
            break;
        }
    }

    // Set default serif font family
    for family in [
        "Times New Roman",
        "Times",
        "Liberation Serif",
        "DejaVu Serif",
    ] {
        if families.contains(family) {
            fontdb.set_serif_family(family);
            break;
        }
    }
}

pub fn register_font_directory(dir: &str) {
    let mut font_system = FONT_SYSTEM
        .lock()
        .expect("Failed to acquire lock on FONT_SYSTEM");
    let fontdb = font_system.db_mut();
    fontdb.load_fonts_dir(dir);
    setup_default_fonts(fontdb);
}

type CosmicCacheKey = (cosmic_text::CacheKey, [u8; 4]);

#[derive(Clone, Debug)]
pub struct CosmicTextRasterizer;

impl TextRasterizer for CosmicTextRasterizer {
    type CacheKey = CosmicCacheKey;

    fn rasterize(
        &self,
        dimensions: CanvasDimensions,
        config: &TextRasterizationConfig,
        cached_glyphs: &HashMap<Self::CacheKey, GlyphBBoxAndAtlasCoords>,
    ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerWgpuError> {
        let mut font_system = FONT_SYSTEM
            .lock()
            .expect("Failed to acquire lock on FONT_SYSTEM");
        let mut cache = SWASH_CACHE
            .lock()
            .expect("Failed to acquire lock on SWASH_CACHE");

        // Build image cache
        let mut next_cache: HashMap<CosmicCacheKey, GlyphImage<CosmicCacheKey>> = HashMap::new();

        // Build cosmic-text Buffer
        let mut buffer = Buffer::new(
            &mut font_system,
            Metrics::new(config.font_size, config.font_size),
        );
        let family = match config.font.to_lowercase().as_str() {
            "serif" => Family::Serif,
            "sans serif" | "sans-serif" => Family::SansSerif,
            "cursive" => Family::Cursive,
            "fantasy" => Family::Fantasy,
            "monospace" => Family::Monospace,
            _ => Family::Name(config.font.as_str()),
        };
        let weight = match config.font_weight {
            FontWeightSpec::Name(FontWeightNameSpec::Bold) => Weight::BOLD,
            FontWeightSpec::Name(FontWeightNameSpec::Normal) => Weight::NORMAL,
            FontWeightSpec::Number(w) => Weight(*w as u16),
        };

        buffer.set_text(
            &mut font_system,
            config.text,
            Attrs::new().family(family).weight(weight),
            Shaping::Advanced,
        );

        buffer.set_size(
            &mut font_system,
            Some(dimensions.size[0]),
            Some(dimensions.size[1]),
        );
        buffer.shape_until_scroll(&mut font_system, false);

        let (buffer_width, line_y, buffer_height) = measure(&buffer);

        let text_color = [
            (config.color[0] * 255.0).round() as u8,
            (config.color[1] * 255.0).round() as u8,
            (config.color[2] * 255.0).round() as u8,
            (config.color[3] * 255.0).round() as u8,
        ];

        // Initialize glyphs
        let mut glyphs: Vec<(GlyphImage<CosmicCacheKey>, PhysicalGlyphPosition)> = Vec::new();

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
                    glyphs.push((glyph_image.without_image(), phys_pos));
                } else if let Some(glyph_bbox_and_altas_coords) = cached_glyphs.get(&cache_key) {
                    // Glyph already rasterized by a prior call to rasterize(), so we can just
                    // store the cache key and position info.
                    glyphs.push((
                        GlyphImage {
                            cache_key,
                            image: None,
                            bbox: glyph_bbox_and_altas_coords.bbox,
                        },
                        phys_pos,
                    ));
                } else {
                    // We need to rasterize glyph and write it to next_atlas
                    let Some(image) = cache
                        .get_image(&mut font_system, physical_glyph.cache_key)
                        .as_ref()
                    else {
                        return Err(AvengerWgpuError::ImageAllocationError(
                            "Failed to create glyph image".to_string(),
                        ));
                    };

                    let width = image.placement.width as usize;
                    let height = image.placement.height as usize;
                    let should_rasterize = width > 0 && height > 0;

                    if !should_rasterize {
                        continue;
                    }

                    let img = match image.content {
                        SwashContent::Color => {
                            // Image is rgba (like an emoji)
                            let Some(img) = image::RgbaImage::from_vec(
                                width as u32,
                                height as u32,
                                image.data.clone(),
                            ) else {
                                return Err(AvengerWgpuError::ImageAllocationError(
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
                                return Err(AvengerWgpuError::ImageAllocationError(
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

                    // Create new glyph image
                    let glyph_image = GlyphImage {
                        cache_key: (physical_glyph.cache_key, text_color),
                        image: Some(img),
                        bbox: GlyphBBox {
                            top: image.placement.top,
                            left: image.placement.left,
                            width: image.placement.width,
                            height: image.placement.height,
                        },
                    };

                    // Update cache
                    next_cache.insert(cache_key, glyph_image.without_image());

                    glyphs.push((glyph_image, phys_pos));
                };
            }
        }

        Ok(TextRasterizationBuffer {
            glyphs,
            buffer_width,
            buffer_height,
            buffer_line_y: line_y,
        })
    }
}

pub fn measure(buffer: &Buffer) -> (f32, f32, f32) {
    let (width, line_y, total_lines) =
        buffer
            .layout_runs()
            .fold((0.0, 0.0, 0usize), |(width, line_y, total_lines), run| {
                (
                    run.line_w.max(width),
                    run.line_y.max(line_y),
                    total_lines + 1,
                )
            });
    (
        width,
        line_y,
        total_lines as f32 * buffer.metrics().line_height,
    )
}
