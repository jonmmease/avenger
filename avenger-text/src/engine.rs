use std::{collections::HashMap, sync::OnceLock};

use crate::{
    error::AvengerTextError,
    font_resolver::FontResolutionOptions,
    math::TextMarkupConfig,
    measurement::{FontMetrics, FontMetricsConfig, TextBounds, TextMeasurementConfig},
    path::{TextPathBuffer, TextPathExtractionConfig, TextPathExtractorImpl},
    pdf::{TextPdfBuffer, TextPdfExtractionConfig, TextPdfExtractorImpl},
    rasterization::{TextRasterCacheKey, TextRasterizationBuffer, TextRasterizationConfig},
    text_line::{TextLineMeasurer, TextLineRasterizer},
    types::TextSyntaxMode,
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
        Self::with_config_and_font_resolution(
            math,
            &FontResolutionOptions {
                load_system_fonts: true,
                ..Default::default()
            },
        )
    }

    pub fn with_config_and_font_resolution(
        math: TextMarkupConfig,
        font_resolution: &FontResolutionOptions,
    ) -> Result<Self, avenger_typst::TypstInitError> {
        let mut config = avenger_typst::TypstEngineConfig::default();
        config.font_config.load_system_fonts = font_resolution.load_system_fonts;
        config.font_config.extra_font_dirs = font_resolution.extra_font_dirs.clone();
        Ok(Self::new(avenger_typst::AvengerTypst::new(config)?, math))
    }

    pub fn with_default_config() -> Result<Self, avenger_typst::TypstInitError> {
        Self::with_config(TextMarkupConfig::default())
    }

    pub fn with_font_resolution(
        font_resolution: &FontResolutionOptions,
    ) -> Result<Self, avenger_typst::TypstInitError> {
        Self::with_config_and_font_resolution(TextMarkupConfig::default(), font_resolution)
    }

    pub fn measure_bounds(
        &self,
        config: &TextMeasurementConfig,
    ) -> Result<TextBounds, AvengerTextError> {
        TextLineMeasurer::new(self.typst.clone(), self.math.clone()).measure_text_bounds(config)
    }

    pub fn measure_bounds_with_plain_fallback(
        &self,
        config: &TextMeasurementConfig,
    ) -> Result<TextBounds, AvengerTextError> {
        self.measure_bounds(config).or_else(|_| {
            let mut plain_config = config.clone();
            plain_config.syntax_mode = TextSyntaxMode::Plain;
            TextLineMeasurer::new(self.typst.clone(), self.math.plain_text())
                .measure_text_bounds(&plain_config)
        })
    }

    pub fn measure_bounds_with_plain_fallback_or_approx(
        &self,
        config: &TextMeasurementConfig,
    ) -> TextBounds {
        self.measure_bounds_with_plain_fallback(config)
            .unwrap_or_else(|_| approximate_text_bounds(config.text, config.font_size))
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

    pub fn rasterize_with_plain_fallback<CacheValue>(
        &self,
        config: &TextRasterizationConfig,
        scale: f32,
        cached_entries: &HashMap<TextRasterCacheKey, CacheValue>,
    ) -> Result<TextRasterizationBuffer<TextRasterCacheKey>, AvengerTextError>
    where
        CacheValue: Clone,
    {
        self.rasterize(config, scale, cached_entries).or_else(|_| {
            let mut plain_config = config.clone();
            plain_config.syntax_mode = TextSyntaxMode::Plain;
            TextLineRasterizer::<CacheValue>::new(self.typst.clone(), self.math.plain_text())
                .rasterize(&plain_config, scale, cached_entries)
        })
    }

    pub fn extract_paths(
        &self,
        config: &TextPathExtractionConfig,
    ) -> Result<TextPathBuffer, AvengerTextError> {
        TextPathExtractorImpl::new(self.typst.clone(), self.math.clone()).extract_text_paths(config)
    }

    pub fn extract_paths_with_plain_fallback(
        &self,
        config: &TextPathExtractionConfig,
    ) -> Result<TextPathBuffer, AvengerTextError> {
        self.extract_paths(config).or_else(|_| {
            let mut plain_config = config.clone();
            plain_config.syntax_mode = TextSyntaxMode::Plain;
            TextPathExtractorImpl::new(self.typst.clone(), self.math.plain_text())
                .extract_text_paths(&plain_config)
        })
    }

    pub fn extract_pdf(
        &self,
        config: &TextPdfExtractionConfig,
    ) -> Result<TextPdfBuffer, AvengerTextError> {
        TextPdfExtractorImpl::new(self.typst.clone(), self.math.clone()).extract_pdf(config)
    }

    pub fn extract_pdf_with_plain_fallback(
        &self,
        config: &TextPdfExtractionConfig,
    ) -> Result<TextPdfBuffer, AvengerTextError> {
        self.extract_pdf(config).or_else(|_| {
            let mut plain_config = config.clone();
            plain_config.syntax_mode = TextSyntaxMode::Plain;
            TextPdfExtractorImpl::new(self.typst.clone(), self.math.plain_text())
                .extract_pdf(&plain_config)
        })
    }
}

fn approximate_text_bounds(text: &str, font_size: f32) -> TextBounds {
    let height = font_size.max(1.0);
    let ascent = height * 0.8;
    let descent = height - ascent;
    TextBounds {
        width: text.chars().count() as f32 * height * 0.6,
        height,
        ascent,
        descent,
        line_height: height,
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
        types::{FontStyle, FontWeight, FontWeightNameSpec, TextSyntaxMode},
    };

    static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
    static STYLE: FontStyle = FontStyle::Normal;
    static COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn engine() -> TextEngine {
        TextEngine::with_default_config().unwrap()
    }

    #[test]
    fn text_syntax_mode_defaults_to_plain() {
        assert_eq!(TextSyntaxMode::default(), TextSyntaxMode::Plain);
    }

    fn measure<'a>(text: &'a String, font: &'a String) -> TextMeasurementConfig<'a> {
        TextMeasurementConfig {
            text,
            font,
            font_size: 14.0,
            font_weight: WEIGHT,
            font_style: STYLE,
            syntax_mode: TextSyntaxMode::TypstMarkup,
        }
    }

    fn paths<'a>(text: &'a String, font: &'a String) -> TextPathExtractionConfig<'a> {
        TextPathExtractionConfig {
            text,
            color: COLOR,
            font,
            font_size: 14.0,
            font_weight: WEIGHT,
            font_style: STYLE,
            limit: f32::INFINITY,
            syntax_mode: TextSyntaxMode::TypstMarkup,
        }
    }

    fn raster<'a>(text: &'a String, font: &'a String) -> TextRasterizationConfig<'a> {
        TextRasterizationConfig {
            text,
            color: COLOR,
            font,
            font_size: 14.0,
            font_weight: WEIGHT,
            font_style: STYLE,
            limit: f32::INFINITY,
            syntax_mode: TextSyntaxMode::TypstMarkup,
        }
    }

    #[test]
    fn top_level_engine_measures_mixed_math_and_named_emoji() {
        let engine = engine();
        let font = "sans-serif".to_string();
        let text = "Revenue #emoji.face $x^2$".to_string();

        let bounds = engine.measure_bounds(&measure(&text, &font)).unwrap();
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
            let bounds = engine.measure_bounds(&measure(&text, &font)).unwrap();
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

        assert_eq!(buffer.entries.len(), 1);
        assert!(buffer.text_bounds.width > 0.0);
        assert_eq!(buffer.entries[0].0.cache_key.text, text);
        assert!(buffer.entries[0].0.image.is_some());
        #[cfg(target_os = "macos")]
        assert!(
            colored_pixel_count(buffer.entries[0].0.image.as_ref().unwrap().as_raw()) > 20,
            "emoji text raster should contain colored pixels on macOS"
        );
    }

    #[cfg(target_os = "macos")]
    fn colored_pixel_count(data: &[u8]) -> usize {
        data.chunks_exact(4)
            .filter(|pixel| {
                let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
                a > 0 && r.abs_diff(g).max(r.abs_diff(b)).max(g.abs_diff(b)) > 16
            })
            .count()
    }

    #[test]
    fn top_level_engine_errors_on_invalid_math() {
        let engine = engine();
        let font = "sans-serif".to_string();
        let text = "before $x^$ after".to_string();

        assert!(engine.measure_bounds(&measure(&text, &font)).is_err());
        assert!(engine.extract_paths(&paths(&text, &font)).is_err());
        assert!(engine
            .rasterize(
                &raster(&text, &font),
                2.0,
                &std::collections::HashMap::<_, ()>::new(),
            )
            .is_err());
    }

    #[test]
    fn top_level_engine_errors_on_unmatched_typst_dollar() {
        let engine = engine();
        let font = "sans-serif".to_string();
        let text = "cost $5".to_string();

        assert!(engine.measure_bounds(&measure(&text, &font)).is_err());
        assert!(engine.extract_paths(&paths(&text, &font)).is_err());
    }

    #[test]
    fn top_level_engine_typst_escaped_dollar_succeeds() {
        let engine = engine();
        let font = "sans-serif".to_string();
        let text = r"cost \$5".to_string();

        let bounds = engine.measure_bounds(&measure(&text, &font)).unwrap();
        assert!(bounds.width > 0.0);

        let buffer = engine.extract_paths(&paths(&text, &font)).unwrap();
        assert_eq!(buffer.plain_runs.len(), 1);
        assert_eq!(buffer.plain_runs[0].text, "cost $5");
    }

    #[test]
    fn top_level_engine_plain_fallback_displays_invalid_math_as_text() {
        let engine = engine();
        let font = "sans-serif".to_string();
        let text = "before $x^$ after".to_string();

        let bounds = engine
            .measure_bounds_with_plain_fallback(&measure(&text, &font))
            .unwrap();
        assert!(bounds.width > 0.0);

        let buffer = engine
            .extract_paths_with_plain_fallback(&paths(&text, &font))
            .unwrap();
        assert!(buffer.items.is_empty());
        assert_eq!(buffer.plain_runs.len(), 1);
        assert_eq!(buffer.plain_runs[0].text, text);

        let raster = engine
            .rasterize_with_plain_fallback(
                &raster(&text, &font),
                2.0,
                &std::collections::HashMap::<_, ()>::new(),
            )
            .unwrap();
        assert_eq!(raster.entries.len(), 1);
    }

    #[test]
    fn top_level_engine_accepts_literal_dollars() {
        let engine = engine();
        let font = "sans-serif".to_string();

        for text in ["cost \\$5", "cost $5"] {
            let text = text.to_string();
            let mut config = measure(&text, &font);
            config.syntax_mode = TextSyntaxMode::Plain;
            let bounds = engine.measure_bounds(&config).unwrap();
            assert!(bounds.width > 0.0);
        }
    }
}
