use std::hint::black_box;

#[cfg(feature = "typst")]
fn run_text_line() {
    use std::collections::HashMap;

    use avenger_text::measurement::TextMeasurementConfig;
    use avenger_text::rasterization::{TextRasterCacheKey, TextRasterizationConfig};
    use avenger_text::types::{FontStyle, FontWeight};
    use avenger_text::default_text_engine;

    let font = "sans-serif".to_string();
    let weight = FontWeight::default();
    let style = FontStyle::default();
    let measurement = TextMeasurementConfig {
        text: "energy $E = mc^2$",
        font: &font,
        font_size: 12.0,
        font_weight: &weight,
        font_style: &style,
    };

    let text_engine = default_text_engine();
    black_box(text_engine.measure_bounds(&measurement));

    let text = measurement.text.to_string();
    let color = [0.1, 0.2, 0.3, 1.0];
    let config = TextRasterizationConfig {
        text: &text,
        color: &color,
        font: &font,
        font_size: 12.0,
        font_weight: &weight,
        font_style: &style,
        limit: f32::INFINITY,
    };

    let cached_entries = HashMap::<TextRasterCacheKey, ()>::new();
    black_box(
        text_engine
            .rasterize(&config, 2.0, &cached_entries)
            .expect("rasterize mixed Typst math text"),
    );
}

fn main() {
    #[cfg(feature = "typst")]
    run_text_line();

    #[cfg(not(feature = "typst"))]
    black_box("baseline");
}
