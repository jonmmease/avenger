use std::collections::HashMap;
use avenger_wgpu::canvas::CanvasDimensions;
use avenger_wgpu::error::AvengerWgpuError;
use avenger_wgpu::marks::text::{GlyphBBoxAndAtlasCoords, TextRasterizationBuffer, TextRasterizationConfig, TextRasterizer};
use crate::log;

#[derive(Clone, Debug)]
pub struct HtmlCanvasTextRasterizer;

impl TextRasterizer for HtmlCanvasTextRasterizer {
    type CacheKey = ();

    fn rasterize(&self, dimensions: CanvasDimensions, config: &TextRasterizationConfig, cached_glyphs: &HashMap<Self::CacheKey, GlyphBBoxAndAtlasCoords>) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerWgpuError> {
        log(&format!("rasterize: {}", config.text));
        Ok(TextRasterizationBuffer {
            glyphs: vec![],
            buffer_width: 0.0,
            buffer_height: 0.0,
            buffer_line_y: 0.0,
        })
    }
}
