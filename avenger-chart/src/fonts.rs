use std::sync::OnceLock;

use avenger_text::{FontResolutionOptions, TextEngine};

pub fn default_font_resolution() -> FontResolutionOptions {
    avenger_text::fonts::default_font_resolution()
}

pub(crate) fn default_chart_text_engine() -> TextEngine {
    static DEFAULT_TEXT_ENGINE: OnceLock<TextEngine> = OnceLock::new();
    DEFAULT_TEXT_ENGINE
        .get_or_init(|| {
            TextEngine::with_font_resolution(&default_font_resolution())
                .expect("failed to initialize chart Typst text engine")
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_text::{
        empty_label_params,
        measurement::TextMeasurementConfig,
        types::{FontStyle, FontWeight, FontWeightNameSpec, TextSyntaxMode},
    };

    #[test]
    fn chart_default_fonts_register_text_and_math_families() {
        let options = default_font_resolution();
        let fontdb = avenger_text::fonts::build_fontdb(&options);

        for family in ["Lato", "DejaVu Sans Mono", "Lete Sans Math"] {
            assert!(
                fontdb.faces().any(|face| face
                    .families
                    .iter()
                    .any(|(candidate, _)| candidate == family)),
                "{family} should be registered"
            );
        }
    }

    #[test]
    fn chart_and_shared_text_defaults_measure_sans_serif_identically() {
        let text = "Axis label 012345".to_string();
        let font = "sans-serif".to_string();
        let params = empty_label_params();
        let config = TextMeasurementConfig {
            text: &text,
            font: &font,
            font_size: 12.0,
            font_weight: FontWeight::Name(FontWeightNameSpec::Normal),
            font_style: FontStyle::Normal,
            syntax_mode: TextSyntaxMode::Plain,
            params: &params,
        };

        let chart_bounds = default_chart_text_engine()
            .measure_bounds(&config)
            .expect("chart default should measure sans-serif");
        let shared_bounds = avenger_text::default_text_engine()
            .measure_bounds(&config)
            .expect("shared default should measure sans-serif");

        assert_eq!(chart_bounds, shared_bounds);
    }
}
