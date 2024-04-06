use std::collections::HashMap;
use std::fmt::format;
use std::hash::{DefaultHasher, Hash, Hasher};
use wasm_bindgen::{JsCast, JsError, JsValue};
use web_sys::{OffscreenCanvas, OffscreenCanvasRenderingContext2d};
use avenger_wgpu::canvas::CanvasDimensions;
use avenger_wgpu::error::AvengerWgpuError;
use avenger_wgpu::marks::text::{GlyphBBox, GlyphBBoxAndAtlasCoords, GlyphImage, PhysicalGlyphPosition, TextRasterizationBuffer, TextRasterizationConfig, TextRasterizer};
use unicode_segmentation::UnicodeSegmentation;
use avenger::marks::text::{FontStyleSpec, FontWeightNameSpec, FontWeightSpec};
use crate::log;

#[derive(Clone, Debug)]
pub struct HtmlCanvasTextRasterizer;

impl TextRasterizer for HtmlCanvasTextRasterizer {
    type CacheKey = u64;

    fn rasterize(&self, dimensions: CanvasDimensions, config: &TextRasterizationConfig, cached_glyphs: &HashMap<Self::CacheKey, GlyphBBoxAndAtlasCoords>) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerWgpuError> {
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

        let font_str = format!("{style} {weight} {}px {}", config.font_size * dimensions.scale, config.font);
        text_context.set_font(&font_str);

        let color = config.color;
        let color_str = JsValue::from_str(&format!(
            "rgba({}, {}, {}, {})",
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
            color[3],
        ));

        // Initialize string container that will hold the full string up through each cluster
        let mut str_so_far = String::new();

        // Initialize glyphs
        let mut glyphs: Vec<(GlyphImage<u64>, PhysicalGlyphPosition)> = Vec::new();

        for cluster in config.text.graphemes(true) {
            // Compute right edge using full string up through this cluster
            str_so_far.push_str(cluster);

            if cluster.chars().all(|c| c.is_whitespace()) {
                // Skip whitespace characters
                continue
            }

            // Compute metrics of cumulative string up to this cluster
            let cumulative_metrics = text_context.measure_text(&str_so_far)?;

            // Compute position of right edge of cumulative string
            let right = cumulative_metrics.width();

            // Compute metrics on this cluster by itself
            let cluster_metrics = text_context.measure_text(cluster)?;
            let cluster_width = cluster_metrics.width();

            // Calculate the actual bounding box dimensions
            let cluster_actual_width = cluster_metrics.actual_bounding_box_right() + cluster_metrics.actual_bounding_box_left();
            let cluster_height = cluster_metrics.actual_bounding_box_ascent() + cluster_metrics.actual_bounding_box_descent();
            let left = right - cluster_width;
            let top = -cluster_metrics.actual_bounding_box_ascent();

            // Create image for glyph
            let glyph_canvas = OffscreenCanvas::new(
                cluster_actual_width.ceil() as u32 + 2,
                cluster_height.ceil() as u32 + 2,
            )?;
            let glyph_context = glyph_canvas.get_context("2d")?.unwrap();
            let glyph_context = glyph_context.dyn_into::<OffscreenCanvasRenderingContext2d>()?;
            glyph_context.set_font(&font_str);
            glyph_context.set_fill_style(&color_str);

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
                0.0, 0.0, glyph_canvas.width() as f64, glyph_canvas.height() as f64
            )?;
            let img = image::RgbaImage::from_raw(
                image_data.width(), image_data.height(), image_data.data().0
            ).expect("Failed to import glyph image");

            // Physical position, relative to start of origin of string
            let phys_pos = PhysicalGlyphPosition {
                x: left as f32,
                y: top as f32,
            };

            // Build cache key concatenating font and cluster
            let cache_key = calculate_cache_key(&font_str, &cluster);

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
            glyphs.push((glyph_image, phys_pos));
        }

        // Compute final buffer metrics
        let full_metrics = text_context.measure_text(config.text)?;
        let buffer_width = full_metrics.actual_bounding_box_left() + full_metrics.actual_bounding_box_right();
        let buffer_height = full_metrics.actual_bounding_box_ascent() + full_metrics.actual_bounding_box_descent();
        let buffer_line_y = full_metrics.actual_bounding_box_ascent();

        Ok(TextRasterizationBuffer {
            glyphs,
            buffer_width: buffer_width as f32,
            buffer_height: buffer_height as f32,
            buffer_line_y: buffer_line_y as f32,
        })
    }
}


fn calculate_cache_key(font_str: &str, cluster: &str) -> u64 {
    let mut s = DefaultHasher::new();
    font_str.hash(&mut s);
    cluster.hash(&mut s);
    s.finish()
}
