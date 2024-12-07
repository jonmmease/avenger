use crate::error::AvengerTextError;
use crate::measurement::html_canvas::GLYPH_CACHE;

use avenger_common::canvas::CanvasDimensions;
use std::collections::HashMap;

use crate::measurement::html_canvas::create_font_string;
use crate::measurement::TextBounds;
use crate::rasterization::{
    GlyphBBox, GlyphData, PhysicalGlyphPosition, TextRasterizationBuffer, TextRasterizationConfig,
    TextRasterizer,
};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::marker::PhantomData;
use unicode_segmentation::UnicodeSegmentation;
use wasm_bindgen::JsCast;
use web_sys::{OffscreenCanvas, OffscreenCanvasRenderingContext2d};

#[derive(Clone, Debug)]
pub struct HtmlCanvasTextRasterizer<CacheValue>
where
    CacheValue: Clone + 'static,
{
    _phantom: PhantomData<CacheValue>,
}

impl<CacheValue> HtmlCanvasTextRasterizer<CacheValue>
where
    CacheValue: Clone + 'static,
{
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<CacheValue> TextRasterizer for HtmlCanvasTextRasterizer<CacheValue>
where
    CacheValue: Clone + 'static,
{
    type CacheKey = u64;
    type CacheValue = CacheValue;

    fn rasterize(
        &self,
        dimensions: &CanvasDimensions,
        config: &TextRasterizationConfig,
        cached_glyphs: &HashMap<Self::CacheKey, CacheValue>,
    ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerTextError> {
        let mut glyph_cache = GLYPH_CACHE
            .lock()
            .expect("Failed to acquire lock on GLYPH_CACHE");

        // Create context for measuring text (we don't draw to this one)
        let offscreen_canvas = OffscreenCanvas::new(400, 400)?;
        let context = offscreen_canvas.get_context("2d")?.unwrap();
        let text_context = context.dyn_into::<OffscreenCanvasRenderingContext2d>()?;

        // Build font string compatible with canvas
        let font_str = create_font_string(&config.to_measurement_config(), dimensions.scale);
        text_context.set_font(&font_str);

        let color = config.color;
        let color_str = format!(
            "rgba({}, {}, {}, {})",
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
            color[3],
        );

        // Initialize string container that will hold the full string up through each cluster
        let mut str_so_far = String::new();

        // Initialize glyphs
        let mut glyphs: Vec<(GlyphData<u64>, PhysicalGlyphPosition)> = Vec::new();

        for cluster in config.text.graphemes(true) {
            // Compute right edge using full string up through this cluster
            str_so_far.push_str(cluster);

            if cluster.chars().all(|c| c.is_whitespace()) {
                // Skip whitespace characters
                continue;
            }

            // Build cache key concatenating font and cluster
            let cache_key = calculate_cache_key(&font_str, &cluster, &color_str);
            // Compute metrics of cumulative string up to this cluster
            let cumulative_metrics = text_context.measure_text(&str_so_far)?;

            // Compute position of right edge of cumulative string
            let right = cumulative_metrics.width();

            // Compute metrics on this cluster by itself
            let cluster_metrics = text_context.measure_text(cluster)?;
            let cluster_width = cluster_metrics.width();

            // Calculate the actual bounding box dimensions
            let cluster_actual_width = cluster_metrics.actual_bounding_box_right()
                + cluster_metrics.actual_bounding_box_left();
            let cluster_height = cluster_metrics.actual_bounding_box_ascent()
                + cluster_metrics.actual_bounding_box_descent();
            let left = right - cluster_width;
            let top = -cluster_metrics.actual_bounding_box_ascent();

            // Physical position, relative to start of origin of string
            let phys_pos = PhysicalGlyphPosition {
                x: left as f32,
                y: top as f32,
            };

            if let Some(glyph_data) = glyph_cache.get(&cache_key) {
                // Glyph has already been rasterized previously, but the image may be needed
                glyphs.push((glyph_data.clone(), phys_pos));
            } else {
                // Extract bbox dimensions
                let canvas_width = cluster_actual_width.ceil() as u32 + 2;
                let canvas_height = cluster_height.ceil() as u32 + 2;
                let draw_x = cluster_metrics.actual_bounding_box_left();
                let draw_y = cluster_metrics.actual_bounding_box_ascent();

                let bbox = GlyphBBox {
                    left: -draw_x as i32,
                    top: 0i32,
                    width: canvas_width,
                    height: canvas_height,
                };

                if cached_glyphs.contains_key(&cache_key) {
                    // Glyph already rasterized by a prior call to rasterize() so we can just
                    // return the cache key and physical position info.
                    glyphs.push((
                        GlyphData {
                            cache_key,
                            image: None,
                            path: None,
                            bbox,
                        },
                        phys_pos,
                    ));
                } else {
                    // Create image for glyph
                    let glyph_canvas = OffscreenCanvas::new(canvas_width, canvas_height)?;
                    let glyph_context = glyph_canvas.get_context("2d")?.unwrap();
                    let glyph_context =
                        glyph_context.dyn_into::<OffscreenCanvasRenderingContext2d>()?;
                    glyph_context.set_font(&font_str);
                    glyph_context.set_fill_style_str(&color_str);

                    // // Debugging, add bbox outline
                    // glyph_context.set_stroke_style(&"red".into());
                    // glyph_context.set_line_width(1.0);
                    // glyph_context.stroke_rect(0.0, 0.0, glyph_canvas.width() as f64, glyph_canvas.height() as f64);

                    // Draw text to canvas
                    glyph_context.fill_text(cluster, draw_x + 1.0, draw_y + 1.0)?;

                    // Convert canvas to image
                    let image_data = glyph_context.get_image_data(
                        0.0,
                        0.0,
                        glyph_canvas.width() as f64,
                        glyph_canvas.height() as f64,
                    )?;
                    let img = image::RgbaImage::from_raw(
                        image_data.width(),
                        image_data.height(),
                        image_data.data().0,
                    )
                    .expect("Failed to import glyph image");

                    let glyph_data = GlyphData {
                        cache_key,
                        image: Some(img),
                        path: None,
                        bbox: GlyphBBox {
                            left: -draw_x as i32,
                            top: 0i32,
                            width: image_data.width(),
                            height: image_data.height(),
                        },
                    };
                    glyph_cache.insert(cache_key, glyph_data.clone());
                    glyphs.push((glyph_data, phys_pos));
                }
            }
        }

        // Compute final buffer metrics
        let full_metrics = text_context.measure_text(config.text)?;
        let buffer_width =
            full_metrics.actual_bounding_box_left() + full_metrics.actual_bounding_box_right();

        // Using font_bounding_box_descent instead of actual_bounding_box_descent
        // results in a better match with cosmic-text
        let ascent = full_metrics.actual_bounding_box_ascent();
        let descent = full_metrics.font_bounding_box_descent();

        let text_bounds = TextBounds {
            width: buffer_width as f32 / dimensions.scale,
            height: (ascent + descent) as f32 / dimensions.scale,
            ascent: ascent as f32 / dimensions.scale,
            descent: descent as f32 / dimensions.scale,
            line_height: (ascent + descent) as f32 / dimensions.scale,
        };
        Ok(TextRasterizationBuffer {
            glyphs,
            text_bounds,
        })
    }
}

fn calculate_cache_key(font_str: &str, cluster: &str, color_str: &str) -> u64 {
    let mut s = DefaultHasher::new();
    font_str.hash(&mut s);
    cluster.hash(&mut s);
    color_str.hash(&mut s);
    s.finish()
}
