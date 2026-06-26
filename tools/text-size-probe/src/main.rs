use std::hint::black_box;

#[cfg(feature = "typst")]
fn run_typst_text() {
    use std::collections::HashMap;

    use avenger_text::measurement::{
        default_text_measurer, TextMeasurementConfig, TextMeasurer,
    };
    use avenger_text::rasterization::{
        default_rasterizer, TextRasterizationConfig, TextRasterizer,
    };
    use avenger_text::types::{FontStyle, FontWeight};

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

    let measurer = default_text_measurer();
    black_box(measurer.measure_text_bounds(&measurement));

    let rasterizer = default_rasterizer();
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

    black_box(
        rasterizer
            .rasterize(&config, 2.0, &HashMap::new())
            .expect("rasterize mixed Typst math text"),
    );
}

fn main() {
    #[cfg(feature = "typst")]
    run_typst_text();

    #[cfg(not(feature = "typst"))]
    black_box("baseline");
}
