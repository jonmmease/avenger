use super::{TextBounds, TextMeasurementConfig, TextMeasurer};
use crate::rasterization::GlyphData;
use crate::types::{FontStyle, FontWeight, FontWeightNameSpec};
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::Mutex;
use wasm_bindgen::JsCast;
use web_sys::{OffscreenCanvas, OffscreenCanvasRenderingContext2d};

lazy_static! {
    // TODO: use LRU cache
    pub(crate) static ref GLYPH_CACHE: Mutex<HashMap<u64, GlyphData<u64>>> =
        Mutex::new(HashMap::new());
}

/// Helper function to create a canvas font string
pub(crate) fn create_font_string(config: &TextMeasurementConfig, scale: f32) -> String {
    let weight = match &config.font_weight {
        FontWeight::Name(FontWeightNameSpec::Bold) => "bold".to_string(),
        FontWeight::Name(FontWeightNameSpec::Normal) => "normal".to_string(),
        FontWeight::Number(w) => (*w as u32).to_string(),
    };

    let style = match &config.font_style {
        FontStyle::Normal => "normal",
        FontStyle::Italic => "italic",
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
    scale: f32,
) -> Result<TextBounds, wasm_bindgen::JsValue> {
    let offscreen_canvas = OffscreenCanvas::new(400, 400)?;
    let context = offscreen_canvas.get_context("2d")?.unwrap();
    let text_context = context.dyn_into::<OffscreenCanvasRenderingContext2d>()?;

    text_context.set_font(font_str);
    let metrics = text_context.measure_text(text)?;

    let width = (metrics.actual_bounding_box_left() + metrics.actual_bounding_box_right()) as f32;
    let ascent = metrics.actual_bounding_box_ascent() as f32;
    let descent = metrics.font_bounding_box_descent() as f32;
    let height = ascent + descent;

    Ok(TextBounds {
        width: width / scale,
        height: height / scale,
        ascent: ascent / scale,
        descent: descent / scale,
        line_height: height / scale,
    })
}

pub struct HtmlCanvasTextMeasurer {}

impl HtmlCanvasTextMeasurer {
    pub fn new() -> Self {
        Self {}
    }
}

impl TextMeasurer for HtmlCanvasTextMeasurer {
    fn measure_text_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
        // Scale always 1.0 for text measurement because we want measurement
        // in base coordinates.
        let scale = 1.0;
        let font_str = create_font_string(config, scale);
        measure_text_with_canvas(&config.text, &font_str, scale)
            .unwrap_or_else(|_e| TextBounds::empty())
    }
}
