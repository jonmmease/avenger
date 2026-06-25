use std::hint::black_box;

#[cfg(feature = "cosmic")]
fn run_cosmic_measurement() {
    use avenger_text::measurement::cosmic::CosmicTextMeasurer;
    use avenger_text::measurement::{TextMeasurementConfig, TextMeasurer};
    use avenger_text::types::{FontStyle, FontWeight};

    let measurer = CosmicTextMeasurer::new();
    let weight = FontWeight::default();
    let style = FontStyle::default();
    let plain = TextMeasurementConfig {
        text: "plain label",
        font: "sans-serif",
        font_size: 12.0,
        font_weight: &weight,
        font_style: &style,
    };
    let math_like = TextMeasurementConfig {
        text: "energy $E = mc^2$",
        font: "sans-serif",
        font_size: 12.0,
        font_weight: &weight,
        font_style: &style,
    };

    black_box(measurer.measure_text_bounds(&plain));
    black_box(measurer.measure_text_bounds(&math_like));
}

#[cfg(feature = "typst-math")]
fn run_typst_math() {
    use avenger_typst::{
        AvengerTypst, MathOutputRequest, MathStringOptions, TypstEngineBackend, TypstEngineConfig,
    };

    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..TypstEngineConfig::default()
    })
    .expect("initialize vendored Typst math engine");

    let mut options = MathStringOptions::default();
    options.outputs = MathOutputRequest {
        paths: true,
        #[cfg(feature = "typst-math-raster")]
        raster: Some(avenger_typst::RasterRequest { scale: 2.0 }),
        #[cfg(not(feature = "typst-math-raster"))]
        raster: None,
        pdf_text_layer: true,
    };

    black_box(
        engine
            .typeset_math_string("plain label", &options)
            .expect("typeset plain string"),
    );
    black_box(
        engine
            .typeset_math_string("energy $E = mc^2$", &options)
            .expect("typeset math string"),
    );
}

#[cfg(all(feature = "cosmic", feature = "typst-math-raster"))]
fn run_mixed_text_rasterization() {
    use std::collections::HashMap;

    use avenger_text::math::{TextMarkupMode, TextMathConfig};
    use avenger_text::rasterization::cosmic::CosmicTextRasterizer;
    use avenger_text::rasterization::math::MathAwareTextRasterizer;
    use avenger_text::rasterization::{TextRasterizationConfig, TextRasterizer};
    use avenger_text::types::{FontStyle, FontWeight};

    let plain = CosmicTextRasterizer::<()>::new();
    let rasterizer = MathAwareTextRasterizer::<_, ()>::with_vendor_typst(
        plain,
        TextMathConfig {
            mode: TextMarkupMode::TypstMathDelimited(Default::default()),
            ..Default::default()
        },
    )
    .expect("initialize math-aware text rasterizer");

    let text = "energy $E = mc^2$".to_string();
    let color = [0.1, 0.2, 0.3, 1.0];
    let font = "sans-serif".to_string();
    let weight = FontWeight::default();
    let style = FontStyle::default();
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
    #[cfg(feature = "cosmic")]
    run_cosmic_measurement();

    #[cfg(feature = "typst-math")]
    run_typst_math();

    #[cfg(all(feature = "cosmic", feature = "typst-math-raster"))]
    run_mixed_text_rasterization();

    #[cfg(not(any(feature = "cosmic", feature = "typst-math")))]
    black_box("plain-none");
}
