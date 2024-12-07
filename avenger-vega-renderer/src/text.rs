use avenger_common::canvas::CanvasDimensions;
use avenger_text::error::AvengerTextError;
use avenger_text::types::{FontStyleSpec, FontWeightNameSpec, FontWeightSpec};
use avenger_wgpu::error::AvengerWgpuError;
use avenger_wgpu::marks::text::GlyphBBoxAndAtlasCoords;
use lazy_static::lazy_static;
use std::collections::HashMap;

use avenger_text::measurement::TextBounds;
use avenger_text::rasterization::{
    GlyphBBox, GlyphImage, PhysicalGlyphPosition, TextRasterizationBuffer, TextRasterizationConfig,
    TextRasterizer,
};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;
use unicode_segmentation::UnicodeSegmentation;
use wasm_bindgen::JsCast;
use web_sys::{OffscreenCanvas, OffscreenCanvasRenderingContext2d};

lazy_static! {
    // TODO: use LRU cache
    static ref GLYPH_CACHE: Mutex<HashMap<u64, GlyphImage<u64>>> = Mutex::new(HashMap::new());
}

#[derive(Clone, Debug)]
pub struct HtmlCanvasTextRasterizer;

impl TextRasterizer for HtmlCanvasTextRasterizer {
    type CacheKey = u64;
    type CacheValue = GlyphBBoxAndAtlasCoords;

    fn rasterize(
        &self,
        dimensions: &CanvasDimensions,
        config: &TextRasterizationConfig,
        cached_glyphs: &HashMap<Self::CacheKey, GlyphBBoxAndAtlasCoords>,
    ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerTextError> {
        let mut glyph_cache = GLYPH_CACHE
            .lock()
            .expect("Failed to acquire lock on GLYPH_CACHE");

        // Create context for measuring text (we don't draw to this one)
        let offscreen_canvas = OffscreenCanvas::new(400, 400)?;
        let context = offscreen_canvas.get_context("2d")?.unwrap();
        let text_context = context.dyn_into::<OffscreenCanvasRenderingContext2d>()?;

        // Build font string compatible with canvas
        let weight = match &config.font_weight {
            FontWeightSpec::Name(FontWeightNameSpec::Bold) => "bold".to_string(),
            FontWeightSpec::Name(FontWeightNameSpec::Normal) => "normal".to_string(),
            FontWeightSpec::Number(w) => (*w as u32).to_string(),
        };

        let style = match &config.font_style {
            FontStyleSpec::Normal => "normal",
            FontStyleSpec::Italic => "italic",
        };

        let font_str = format!(
            "{style} {weight} {}px {}",
            config.font_size * dimensions.scale,
            config.font
        );
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
        let mut glyphs: Vec<(GlyphImage<u64>, PhysicalGlyphPosition)> = Vec::new();

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

            if let Some(glyph_bbox_and_altas_coords) = cached_glyphs.get(&cache_key) {
                // Glyph already rasterized by a prior call to rasterize() within the same atlas, so we can just
                // store the cache key and position info.
                glyphs.push((
                    GlyphImage {
                        cache_key,
                        image: None,
                        bbox: glyph_bbox_and_altas_coords.bbox,
                    },
                    phys_pos,
                ));
            } else if let Some(glyph_image) = glyph_cache.get(&cache_key) {
                // Glyph has already been rasterized previously, but the image may be needed
                glyphs.push((glyph_image.clone(), phys_pos));
            } else {
                // Create image for glyph
                let glyph_canvas = OffscreenCanvas::new(
                    cluster_actual_width.ceil() as u32 + 2,
                    cluster_height.ceil() as u32 + 2,
                )?;
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
                let draw_x = cluster_metrics.actual_bounding_box_left();
                let draw_y = cluster_metrics.actual_bounding_box_ascent();
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

                let glyph_image = GlyphImage {
                    cache_key,
                    image: Some(img),
                    bbox: GlyphBBox {
                        left: -draw_x as i32,
                        top: 0i32,
                        width: image_data.width(),
                        height: image_data.height(),
                    },
                };
                glyph_cache.insert(cache_key, glyph_image.clone());
                glyphs.push((glyph_image, phys_pos));
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
