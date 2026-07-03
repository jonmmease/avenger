use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use crate::{
    error::AvengerTextError,
    font_resolver::FontResolutionOptions,
    math::TextMarkupConfig,
    measurement::{FontMetrics, FontMetricsConfig, TextBounds, TextMeasurementConfig},
    path::{TextPathBuffer, TextPathExtractionConfig, TextPathExtractorImpl},
    pdf::{TextPdfBuffer, TextPdfExtractionConfig, TextPdfExtractorImpl},
    rasterization::{
        TextRasterCacheKey, TextRasterCacheValue, TextRasterizationBuffer, TextRasterizationConfig,
    },
    text_line::{TextLineMeasurer, TextLineRasterizer},
    types::TextSyntaxMode,
};

/// Bound for the engine-level measurement memo; the map is cleared wholesale
/// when it fills. Entries are tiny (a key string set plus `TextBounds`), and
/// interactive chart chrome re-measures the same few hundred labels
/// frame-to-frame, so a simple epoch reset never hurts steady state.
const MEASURE_BOUNDS_CACHE_CAP: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MeasureBoundsCacheKey {
    text: String,
    font: String,
    font_size_bits: u32,
    font_weight: String,
    font_style: String,
    syntax_mode: TextSyntaxMode,
    params: String,
    number_locale: Option<String>,
    number_locale_specs: String,
    datetime_locale: Option<String>,
    datetime_timezone: Option<String>,
    datetime_locale_specs: String,
}

impl MeasureBoundsCacheKey {
    fn new(config: &TextMeasurementConfig) -> Self {
        Self {
            text: config.text.to_string(),
            font: config.font.to_string(),
            font_size_bits: config.font_size.to_bits(),
            font_weight: format!("{:?}", config.font_weight),
            font_style: format!("{:?}", config.font_style),
            syntax_mode: config.syntax_mode,
            params: crate::label_params_fingerprint(config.params),
            number_locale: config.number_locale.map(str::to_string),
            number_locale_specs: config
                .number_locale_specs
                .map(crate::number_locale_specs_fingerprint)
                .unwrap_or_default(),
            datetime_locale: config.datetime_locale.map(str::to_string),
            datetime_timezone: config.datetime_timezone.map(str::to_string),
            datetime_locale_specs: config
                .datetime_locale_specs
                .map(crate::datetime_locale_specs_fingerprint)
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextEngine {
    typst: avenger_typst_label::LabelEngine,
    math: TextMarkupConfig,
    /// Successful `measure_bounds` results memoized across calls. Shared by
    /// engine clones so the process-wide default engines accumulate one memo.
    measure_bounds_cache: Arc<Mutex<HashMap<MeasureBoundsCacheKey, TextBounds>>>,
}

impl TextEngine {
    pub fn new(typst: avenger_typst_label::LabelEngine, math: TextMarkupConfig) -> Self {
        Self {
            typst,
            math,
            measure_bounds_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_config(
        math: TextMarkupConfig,
    ) -> Result<Self, avenger_typst_label::LabelInitError> {
        Self::with_config_and_font_resolution(math, &crate::fonts::default_font_resolution())
    }

    pub fn with_config_and_font_resolution(
        math: TextMarkupConfig,
        font_resolution: &FontResolutionOptions,
    ) -> Result<Self, avenger_typst_label::LabelInitError> {
        let mut options = avenger_typst_label::EngineOptions::default();
        options.fonts.load_system_fonts = font_resolution.load_system_fonts;
        options.fonts.extra_font_dirs = font_resolution.extra_font_dirs.clone();
        options.fonts.registered_fonts = font_resolution.registered_fonts.clone();
        options.fonts.default_sans_serif_family = font_resolution.default_sans_serif_family.clone();
        options.fonts.default_monospace_family = font_resolution.default_monospace_family.clone();
        options.fonts.default_math_family = font_resolution.default_math_family.clone();
        Ok(Self::new(
            avenger_typst_label::LabelEngine::new(options)?,
            math,
        ))
    }

    pub fn with_default_config() -> Result<Self, avenger_typst_label::LabelInitError> {
        Self::with_config(TextMarkupConfig::default())
    }

    pub fn with_font_resolution(
        font_resolution: &FontResolutionOptions,
    ) -> Result<Self, avenger_typst_label::LabelInitError> {
        Self::with_config_and_font_resolution(TextMarkupConfig::default(), font_resolution)
    }

    pub fn measure_bounds(
        &self,
        config: &TextMeasurementConfig,
    ) -> Result<TextBounds, AvengerTextError> {
        let key = MeasureBoundsCacheKey::new(config);
        if let Some(bounds) = self
            .measure_bounds_cache
            .lock()
            .expect("measure bounds cache lock poisoned")
            .get(&key)
        {
            return Ok(bounds.clone());
        }
        let bounds = TextLineMeasurer::new(self.typst.clone(), self.math.clone())
            .measure_text_bounds(config)?;
        let mut cache = self
            .measure_bounds_cache
            .lock()
            .expect("measure bounds cache lock poisoned");
        if cache.len() >= MEASURE_BOUNDS_CACHE_CAP {
            cache.clear();
        }
        cache.insert(key, bounds.clone());
        Ok(bounds)
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
        CacheValue: TextRasterCacheValue,
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
        CacheValue: TextRasterCacheValue,
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
        LabelParamValue, LabelParams,
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
            params: crate::empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
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
            params: crate::empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
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
            params: crate::empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        }
    }

    fn series_name_params(name: &str) -> LabelParams {
        let mut params = LabelParams::default();
        params.insert(
            "series_name".to_string(),
            LabelParamValue::Str(name.to_string()),
        );
        params
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

    #[test]
    fn top_level_engine_resolves_params_for_measurement_and_paths() {
        let engine = engine();
        let font = "sans-serif".to_string();
        let text = "#series_name $x + 1$".to_string();
        let params = series_name_params("Revenue");

        assert!(
            engine.measure_bounds(&measure(&text, &font)).is_err(),
            "markup parameter should be required in Typst syntax mode"
        );

        let mut measurement = measure(&text, &font);
        measurement.params = &params;
        let bounds = engine.measure_bounds(&measurement).unwrap();
        assert!(bounds.width > 0.0);

        let mut path_config = paths(&text, &font);
        path_config.params = &params;
        let buffer = engine.extract_paths(&path_config).unwrap();
        assert!(buffer
            .plain_runs
            .iter()
            .any(|run| run.text.contains("Revenue")));
        assert!(buffer
            .plain_runs
            .iter()
            .all(|run| !run.text.contains("#series_name")));
        assert!(buffer
            .items
            .iter()
            .any(|item| item.kind == TextPathKind::MathGlyph));
    }

    #[test]
    fn top_level_engine_raster_cache_key_separates_params() {
        let engine = engine();
        let font = "sans-serif".to_string();
        let text = "#series_name".to_string();
        let params_a = series_name_params("Revenue");
        let params_b = series_name_params("Cost");

        let mut config_a = raster(&text, &font);
        config_a.params = &params_a;
        let mut config_b = raster(&text, &font);
        config_b.params = &params_b;

        let buffer_a = engine
            .rasterize(&config_a, 2.0, &std::collections::HashMap::<_, ()>::new())
            .unwrap();
        let buffer_b = engine
            .rasterize(&config_b, 2.0, &std::collections::HashMap::<_, ()>::new())
            .unwrap();

        assert_eq!(buffer_a.entries.len(), 1);
        assert_eq!(buffer_b.entries.len(), 1);
        assert_ne!(
            buffer_a.entries[0].0.cache_key.params,
            buffer_b.entries[0].0.cache_key.params
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
