use super::{TextBounds, TextMeasurementConfig, TextMeasurer};
use crate::rasterization::GlyphImage;
use crate::types::{FontStyleSpec, FontWeightNameSpec, FontWeightSpec};
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::Mutex;
use wasm_bindgen::JsCast;
use web_sys::{OffscreenCanvas, OffscreenCanvasRenderingContext2d};

lazy_static! {
    // TODO: use LRU cache
    pub(crate) static ref GLYPH_CACHE: Mutex<HashMap<u64, GlyphImage<u64>>> = Mutex::new(HashMap::new());
}

/// Helper function to create a canvas font string
pub(crate) fn create_font_string(config: &TextMeasurementConfig, scale: f32) -> String {
    let weight = match &config.font_weight {
        FontWeightSpec::Name(FontWeightNameSpec::Bold) => "bold".to_string(),
        FontWeightSpec::Name(FontWeightNameSpec::Normal) => "normal".to_string(),
        FontWeightSpec::Number(w) => (*w as u32).to_string(),
    };

    let style = match &config.font_style {
        FontStyleSpec::Normal => "normal",
        FontStyleSpec::Italic => "italic",
    };

    format!(
        "{style} {weight} {}px {}",
        config.font_size * scale,
        config.font
    )
}

/// Measures text using HTML Canvas
fn measure_text_with_canvas(
    text: &str,
    font_str: &str,
) -> Result<TextBounds, wasm_bindgen::JsValue> {
    let offscreen_canvas = OffscreenCanvas::new(400, 400)?;
    let context = offscreen_canvas.get_context("2d")?.unwrap();
    let text_context = context.dyn_into::<OffscreenCanvasRenderingContext2d>()?;

    text_context.set_font(font_str);
    let metrics = text_context.measure_text(text)?;

    let width = metrics.actual_bounding_box_left() + metrics.actual_bounding_box_right();
    let ascent = metrics.actual_bounding_box_ascent();
    let descent = metrics.font_bounding_box_descent();
    let height = ascent + descent;

    Ok(TextBounds {
        width: width as f32,
        height: height as f32,
        ascent: ascent as f32,
        descent: descent as f32,
        line_height: height as f32,
    })
}

pub struct HtmlCanvasTextMeasurer {}

impl HtmlCanvasTextMeasurer {
    pub fn new() -> Self {
        Self {}
    }
}

impl TextMeasurer for HtmlCanvasTextMeasurer {
    fn measure_text_bounds(
        &self,
        config: &TextMeasurementConfig,
        dimensions: &[f32; 2],
    ) -> TextBounds {
        let scale = dimensions[1];
        let font_str = create_font_string(config, scale);

        match measure_text_with_canvas(&config.text, &font_str) {
            Ok(mut bounds) => {
                // Scale the bounds back to logical pixels
                bounds.width /= scale;
                bounds.height /= scale;
                bounds.ascent /= scale;
                bounds.descent /= scale;
                bounds.line_height /= scale;
                bounds
            }
            Err(_) => TextBounds {
                width: 0.0,
                height: 0.0,
                ascent: 0.0,
                descent: 0.0,
                line_height: 0.0,
            },
        }
    }
}
