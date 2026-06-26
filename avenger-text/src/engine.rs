use std::{collections::HashMap, sync::OnceLock};

use crate::{
    error::AvengerTextError,
    math::TextMarkupConfig,
    measurement::{FontMetrics, FontMetricsConfig, TextBounds, TextMeasurementConfig},
    path::{TextPathBuffer, TextPathExtractionConfig, TextPathExtractorImpl},
    rasterization::{TextRasterCacheKey, TextRasterizationBuffer, TextRasterizationConfig},
    text_line::{TextLineMeasurer, TextLineRasterizer},
};

#[derive(Debug, Clone)]
pub struct TextEngine {
    typst: avenger_typst::AvengerTypst,
    math: TextMarkupConfig,
}

impl TextEngine {
    pub fn new(typst: avenger_typst::AvengerTypst, math: TextMarkupConfig) -> Self {
        Self { typst, math }
    }

    pub fn with_config(math: TextMarkupConfig) -> Result<Self, avenger_typst::TypstInitError> {
        Ok(Self::new(
            avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig {
                backend: avenger_typst::TypstEngineBackend::OwnedTypst,
                ..avenger_typst::TypstEngineConfig::default()
            })?,
            math,
        ))
    }

    pub fn with_default_config() -> Result<Self, avenger_typst::TypstInitError> {
        Self::with_config(TextMarkupConfig::default())
    }

    pub fn measure_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
        TextLineMeasurer::new(self.typst.clone(), self.math.clone()).measure_text_bounds(config)
    }

    pub fn font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics {
        TextLineMeasurer::new(self.typst.clone(), self.math.clone()).measure_font_metrics(config)
    }

    pub fn rasterize<CacheValue>(
        &self,
        config: &TextRasterizationConfig,
        scale: f32,
        cached_entries: &HashMap<TextRasterCacheKey, CacheValue>,
    ) -> Result<TextRasterizationBuffer<TextRasterCacheKey>, AvengerTextError>
    where
        CacheValue: Clone,
    {
        TextLineRasterizer::<CacheValue>::new(self.typst.clone(), self.math.clone()).rasterize(
            config,
            scale,
            cached_entries,
        )
    }

    pub fn extract_paths(
        &self,
        config: &TextPathExtractionConfig,
    ) -> Result<TextPathBuffer, AvengerTextError> {
        TextPathExtractorImpl::new(self.typst.clone(), self.math.clone()).extract_text_paths(config)
    }
}

pub fn default_text_engine() -> TextEngine {
    static DEFAULT_TEXT_ENGINE: OnceLock<TextEngine> = OnceLock::new();
    DEFAULT_TEXT_ENGINE
        .get_or_init(|| {
            TextEngine::with_default_config().expect("failed to initialize Typst text engine")
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        path::{TextPathExtractionConfig, TextPathKind},
        rasterization::TextRasterizationConfig,
        types::{FontStyle, FontWeight, FontWeightNameSpec},
    };

    static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
    static STYLE: FontStyle = FontStyle::Normal;
    static COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn engine() -> TextEngine {
        TextEngine::with_default_config().unwrap()
    }

    fn measure<'a>(text: &'a String, font: &'a String) -> TextMeasurementConfig<'a> {
        TextMeasurementConfig {
            text,
            font,
            font_size: 14.0,
            font_weight: &WEIGHT,
            font_style: &STYLE,
        }
    }

    fn paths<'a>(text: &'a String, font: &'a String) -> TextPathExtractionConfig<'a> {
        TextPathExtractionConfig {
            text,
            color: &COLOR,
            font,
            font_size: 14.0,
            font_weight: &WEIGHT,
            font_style: &STYLE,
            limit: f32::INFINITY,
        }
    }

    fn raster<'a>(text: &'a String, font: &'a String) -> TextRasterizationConfig<'a> {
        TextRasterizationConfig {
            text,
            color: &COLOR,
            font,
            font_size: 14.0,
            font_weight: &WEIGHT,
            font_style: &STYLE,
            limit: f32::INFINITY,
        }
    }

    #[test]
    fn top_level_engine_measures_mixed_math_and_named_emoji() {
        let engine = engine();
        let font = "sans-serif".to_string();
        let text = "Revenue #emoji.face $x^2$".to_string();

        let bounds = engine.measure_bounds(&measure(&text, &font));
        assert!(bounds.width > 0.0);
        assert!(bounds.height >= 14.0);

        let buffer = engine.extract_paths(&paths(&text, &font)).unwrap();
        assert!(buffer
            .items
            .iter()
            .any(|item| item.kind == TextPathKind::MathGlyph));
        assert!(buffer
            .plain_runs
            .iter()
            .any(|run| run.text.contains("Revenue ")));
        assert!(buffer.plain_runs.iter().any(|run| run.text.contains('😀')));
        assert!(buffer
            .plain_runs
            .iter()
            .all(|run| !run.text.contains("#emoji")));
    }

    #[test]
    fn top_level_engine_measures_bidi_and_complex_script_text() {
        let engine = engine();
        let font = "sans-serif".to_string();
        let samples = [("ABC שלום", "שלום"), ("नमस्ते data", "नमस्ते")];

        for (sample, expected_text) in samples {
            let text = sample.to_string();
            let bounds = engine.measure_bounds(&measure(&text, &font));
            assert!(bounds.width > 0.0, "{sample} should have positive width");
            assert!(bounds.height >= 14.0, "{sample} should have line height");

            let buffer = engine.extract_paths(&paths(&text, &font)).unwrap();
            assert!(
                buffer
                    .plain_runs
                    .iter()
                    .any(|run| run.text.contains(expected_text)),
                "{sample} should preserve native text content in path extraction"
            );
        }
    }

    #[test]
    fn top_level_engine_rasterizes_whole_line_with_math_and_emoji() {
        let engine = engine();
        let font = "sans-serif".to_string();
        let text = "Hi #emoji.face $x$".to_string();
        let buffer = engine
            .rasterize(
                &raster(&text, &font),
                2.0,
                &std::collections::HashMap::<_, ()>::new(),
            )
            .unwrap();

        assert_eq!(buffer.glyphs.len(), 1);
        assert!(buffer.text_bounds.width > 0.0);
        assert_eq!(buffer.glyphs[0].0.cache_key.text, text);
        assert!(buffer.glyphs[0].0.image.is_some());
    }
}
