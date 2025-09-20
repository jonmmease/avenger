//! Text measurement utilities for layout

use avenger_text::measurement::cosmic::CosmicTextMeasurer;
use avenger_text::measurement::{TextMeasurementConfig, TextMeasurer};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec};

/// Measure the actual text to get accurate bounds
/// Returns (height, width) for the measured text
pub(crate) fn measure_text(text: &str, font_size: f32, font_family: &str) -> (f32, f32) {
    let measurer = CosmicTextMeasurer::new();

    let config = TextMeasurementConfig {
        text,
        font: font_family,
        font_size,
        font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
        font_style: &FontStyle::Normal,
    };

    let bounds = measurer.measure_text_bounds(&config);

    (bounds.line_height, bounds.width)
}
