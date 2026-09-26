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
    text_edit::ShapedLine,
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
    number_format: usize,
    datetime_format: usize,
}

impl MeasureBoundsCacheKey {
    fn new(
        config: &TextMeasurementConfig,
        number_format: Option<&crate::NumberFormatBinding>,
        datetime_format: Option<&crate::DateTimeFormatBinding>,
    ) -> Self {
        Self {
            text: config.text.to_string(),
            font: config.font.to_string(),
            font_size_bits: config.font_size.to_bits(),
            font_weight: format!("{:?}", config.font_weight),
            font_style: format!("{:?}", config.font_style),
            syntax_mode: config.syntax_mode,
            params: crate::label_params_fingerprint(config.params),
            number_format: config
                .number_format
                .or(number_format)
                .map(crate::NumberFormatBinding::cache_id)
                .unwrap_or_default(),
            datetime_format: config
                .datetime_format
                .or(datetime_format)
                .map(crate::DateTimeFormatBinding::cache_id)
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
        options.fonts.missing_font = font_resolution.missing_font;
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

    /// Set the provider and settings used by numeric labels.
    pub fn with_number_formatting(mut self, binding: crate::NumberFormatBinding) -> Self {
        self.typst = self.typst.with_number_formatting(binding);
        self
    }

    /// Binding inherited by labels without their own number settings.
    pub fn number_format(&self) -> Option<&crate::NumberFormatBinding> {
        self.typst.number_format()
    }

    /// Set the provider and settings used by temporal labels.
    pub fn with_datetime_formatting(mut self, binding: crate::DateTimeFormatBinding) -> Self {
        self.typst = self.typst.with_datetime_formatting(binding);
        self
    }

    /// Binding inherited by labels without their own datetime settings.
    pub fn datetime_format(&self) -> Option<&crate::DateTimeFormatBinding> {
        self.typst.datetime_format()
    }

    pub fn measure_bounds(
        &self,
        config: &TextMeasurementConfig,
    ) -> Result<TextBounds, AvengerTextError> {
        let key = MeasureBoundsCacheKey::new(config, self.number_format(), self.datetime_format());
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

    /// Measure the same overflow layout used by raster, path, and PDF output.
    pub fn measure_bounds_with_limit(
        &self,
        config: &TextMeasurementConfig,
        limit: f32,
    ) -> Result<TextBounds, AvengerTextError> {
        // Validate the complete source even when only a prefix will be displayed.
        let full = self.measure_bounds(config)?;
        if !(limit.is_finite() && limit > 0.0 && full.width > limit) {
            return Ok(full);
        }
        let text = crate::measurement::prepare_text_to_limit_with(
            config.text,
            config.syntax_mode,
            limit,
            |text| {
                self.measure_bounds(&TextMeasurementConfig {
                    text,
                    ..config.clone()
                })
                .map(|bounds| bounds.width)
            },
        )?;
        let mut bounds = self.measure_bounds(&TextMeasurementConfig {
            text: &text,
            ..config.clone()
        })?;
        crate::measurement::apply_text_limit(&mut bounds, config.syntax_mode, limit);
        Ok(bounds)
    }

    pub fn measure_bounds_with_limit_or_approx(
        &self,
        config: &TextMeasurementConfig,
        limit: f32,
    ) -> TextBounds {
        self.measure_bounds_with_limit(config, limit)
            .or_else(|error| {
                if !error.allows_plain_fallback() {
                    return Err(error);
                }
                self.measure_bounds_with_limit(
                    &TextMeasurementConfig {
                        syntax_mode: TextSyntaxMode::Plain,
                        ..config.clone()
                    },
                    limit,
                )
            })
            .unwrap_or_else(|_| {
                let mut bounds = approximate_text_bounds(config.text, config.font_size);
                if limit.is_finite() && limit > 0.0 {
                    bounds.width = bounds.width.min(limit);
                }
                bounds
            })
    }

    pub fn measure_bounds_with_plain_fallback(
        &self,
        config: &TextMeasurementConfig,
    ) -> Result<TextBounds, AvengerTextError> {
        self.measure_bounds(config).or_else(|error| {
            if !error.allows_plain_fallback() {
                return Err(error);
            }
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

    /// Read metrics from the resolved font face. Use `FontMetrics::fallback`
    /// explicitly when approximate proportions are acceptable.
    pub fn font_metrics(
        &self,
        config: &FontMetricsConfig,
    ) -> Result<FontMetrics, AvengerTextError> {
        TextLineMeasurer::new(self.typst.clone(), self.math.clone()).measure_font_metrics(config)
    }

    /// Shape one editable plain-text line and retain source byte geometry.
    pub fn shape_line(
        &self,
        config: &TextMeasurementConfig,
    ) -> Result<ShapedLine, AvengerTextError> {
        crate::text_edit::shaped_line::shape_line(&self.typst, &self.math, config)
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
        self.rasterize(config, scale, cached_entries)
            .or_else(|error| {
                if !error.allows_plain_fallback() {
                    return Err(error);
                }
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
        self.extract_paths(config).or_else(|error| {
            if !error.allows_plain_fallback() {
                return Err(error);
            }
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
        self.extract_pdf(config).or_else(|error| {
            if !error.allows_plain_fallback() {
                return Err(error);
            }
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
        TextEngine::with_default_config()
            .unwrap()
            .with_datetime_formatting(crate::DateTimeFormatBinding::new(
                avenger_format_datetime_d3::D3DateTimeFormatProvider,
                avenger_format_datetime_d3::D3DateTimeFormatConfig::new(),
            ))
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
            number_format: None,
            datetime_format: None,
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
            number_format: None,
            datetime_format: None,
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
            number_format: None,
            datetime_format: None,
        }
    }

    fn pdf_config<'a>(config: &TextPathExtractionConfig<'a>) -> TextPdfExtractionConfig<'a> {
        TextPdfExtractionConfig {
            text: config.text,
            color: config.color,
            font: config.font,
            font_size: config.font_size,
            font_weight: config.font_weight,
            font_style: config.font_style,
            limit: config.limit,
            syntax_mode: config.syntax_mode,
            params: config.params,
            number_format: config.number_format,
            datetime_format: config.datetime_format,
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
    fn formatter_bindings_invalidate_measurement_and_raster_caches() {
        use avenger_format::*;
        #[derive(Debug)]
        struct Provider;
        #[derive(Debug)]
        struct Prepared(String);
        impl NumberFormatProvider for Provider {
            type Config = String;
            fn prepare(
                &self,
                config: &String,
                _: &str,
            ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
                Ok(Arc::new(Prepared(config.clone())))
            }
        }
        impl DateTimeFormatProvider for Provider {
            type Config = String;
            fn prepare_naive(
                &self,
                config: &String,
                _: &str,
            ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError> {
                Ok(Arc::new(Prepared(config.clone())))
            }
            fn prepare_zoned(
                &self,
                config: &String,
                _: &str,
            ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError> {
                Ok(Arc::new(Prepared(config.clone())))
            }
        }
        impl PreparedNumberFormatter for Prepared {
            fn format(&self, _: f64) -> FormattedNumber {
                FormattedNumber::plain(self.0.clone())
            }
        }
        impl PreparedCivilDateTimeFormatter for Prepared {
            fn format(&self, _: NaiveDateTimeInput) -> Result<String, DateTimeFormatError> {
                Ok(self.0.clone())
            }
        }
        impl PreparedInstantFormatter for Prepared {
            fn format(&self, _: ZonedDateTimeInput) -> Result<String, DateTimeFormatError> {
                Ok(self.0.clone())
            }
        }
        let configure = |value: &str| {
            engine()
                .with_number_formatting(NumberFormatBinding::new(Provider, value.to_owned()))
                .with_datetime_formatting(DateTimeFormatBinding::new(Provider, value.to_owned()))
        };
        let first = configure("1");
        let second = configure("100000000");
        let params = LabelParams::from([(
            "value".into(),
            LabelParamValue::UtcDateTime(chrono::DateTime::UNIX_EPOCH),
        )]);
        let font = "sans-serif".to_string();
        for source in ["#numfmt(42, \"custom\")", "#datefmt(value, \"custom\")"] {
            let text = source.to_string();
            let mut measurement = measure(&text, &font);
            measurement.params = &params;
            let mut raster = raster(&text, &font);
            raster.params = &params;
            let first_bounds = first.measure_bounds(&measurement).unwrap();
            let first_raster = first
                .rasterize(&raster, 1.0, &HashMap::<_, ()>::new())
                .unwrap();
            let cache = HashMap::from([(
                first_raster.entries[0].0.cache_key.clone(),
                crate::rasterization::CachedTextRasterization {
                    entries: first_raster.entries.clone(),
                    text_bounds: first_raster.text_bounds.clone(),
                },
            )]);
            let second_bounds = second.measure_bounds(&measurement).unwrap();
            assert!(second_bounds.width > first_bounds.width * 2.0);
            assert_eq!(
                first.clone().measure_bounds(&measurement).unwrap(),
                first_bounds
            );
            let second_raster = second.rasterize(&raster, 1.0, &cache).unwrap();
            assert_ne!(
                first_raster.entries[0].0.cache_key,
                second_raster.entries[0].0.cache_key
            );
            assert!(second_raster.entries[0].0.image.is_some());
            assert_eq!(second_raster.text_bounds, second_bounds);
            measurement.number_format = second.number_format();
            measurement.datetime_format = second.datetime_format();
            raster.number_format = second.number_format();
            raster.datetime_format = second.datetime_format();
            assert_eq!(first.measure_bounds(&measurement).unwrap(), second_bounds);
            let overridden = first.rasterize(&raster, 1.0, &cache).unwrap();
            assert_eq!(
                overridden.entries[0].0.cache_key,
                second_raster.entries[0].0.cache_key
            );
            assert_eq!(overridden.text_bounds, second_raster.text_bounds);
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
    fn parameter_updates_invalidate_measurement_and_raster_caches() {
        use crate::rasterization::CachedTextRasterization;

        let font = "Lato".to_string();
        let mut cases = vec![(
            "#series_name",
            series_name_params("Revenue"),
            series_name_params("Cost"),
        )];
        for year in [1600, 2500] {
            for utc in [false, true] {
                let params = |month| {
                    let value = chrono::NaiveDate::from_ymd_opt(year, month, 1)
                        .unwrap()
                        .and_hms_opt(0, 0, 0)
                        .unwrap();
                    LabelParams::from([(
                        "value".to_string(),
                        if utc {
                            LabelParamValue::UtcDateTime(value.and_utc())
                        } else {
                            LabelParamValue::DateTime(value)
                        },
                    )])
                };
                cases.push((r#"#datefmt(value, "%B")"#, params(1), params(9)));
            }
        }
        for (source, params_a, params_b) in cases {
            let engine = engine();
            let text = source.to_string();
            let mut measurement = measure(&text, &font);
            measurement.params = &params_a;
            let bounds_a = engine.measure_bounds(&measurement).unwrap();
            measurement.params = &params_b;
            let bounds_b = engine.measure_bounds(&measurement).unwrap();
            assert_ne!(bounds_a.width, bounds_b.width);
            assert_eq!(
                bounds_b,
                self::engine().measure_bounds(&measurement).unwrap()
            );

            let mut config = raster(&text, &font);
            config.params = &params_a;
            let buffer_a = engine
                .rasterize(&config, 2.0, &HashMap::<_, ()>::new())
                .unwrap();
            let cache = HashMap::from([(
                buffer_a.entries[0].0.cache_key.clone(),
                CachedTextRasterization {
                    entries: buffer_a.entries,
                    text_bounds: buffer_a.text_bounds,
                },
            )]);
            config.params = &params_b;
            let cached = engine.rasterize(&config, 2.0, &cache).unwrap();
            let fresh = engine
                .rasterize(&config, 2.0, &HashMap::<_, ()>::new())
                .unwrap();
            assert_eq!(cached.text_bounds, fresh.text_bounds);
            assert_eq!(cached.entries[0].0.image, fresh.entries[0].0.image);
        }
    }

    #[test]
    fn rich_width_limits_preserve_source_and_share_bounds_across_outputs() {
        let engine = engine();
        let font = "Lato".to_string();
        for source in [
            "#series_name",
            "*A long bold label* $x^2$",
            "#strong[A long label]",
        ] {
            let text = source.to_string();
            let params = series_name_params("An expanded parameter label");
            let mut measure = measure(&text, &font);
            measure.params = &params;
            let mut raster_config = raster(&text, &font);
            raster_config.params = &params;
            raster_config.limit = 35.0;
            let mut path_config = paths(&text, &font);
            path_config.params = &params;
            path_config.limit = 35.0;
            let expected = engine.measure_bounds_with_limit(&measure, 35.0).unwrap();
            assert_eq!(expected.width, 35.0);
            let raster = engine
                .rasterize(&raster_config, 2.0, &HashMap::<_, ()>::new())
                .unwrap();
            assert_eq!(raster.text_bounds, expected);
            for (entry, position) in &raster.entries {
                assert_eq!(entry.cache_key.text, source);
                assert!(position.x + entry.bbox.width as f32 / 2.0 <= 35.001);
            }
            let path = engine.extract_paths(&path_config).unwrap();
            assert_eq!(path.bounds, expected);
            assert_eq!(path.clip_width, Some(35.0));
            let pdf = engine.extract_pdf(&pdf_config(&path_config)).unwrap();
            assert_eq!(pdf.bounds, expected);
            assert_eq!(pdf.clip_width, Some(35.0));
            assert!(!pdf.semantic_text.contains("#series_name"));
            raster_config.limit = 25.0;
            let narrower = engine
                .rasterize(&raster_config, 2.0, &HashMap::<_, ()>::new())
                .unwrap();
            assert_ne!(
                raster.entries[0].0.cache_key,
                narrower.entries[0].0.cache_key
            );
        }
    }

    #[test]
    fn source_and_font_errors_survive_all_plain_fallback_entry_points() {
        let font = "Lato".to_string();
        let text = "eight!!!".to_string();
        let mut markup = TextMarkupConfig::default();
        markup.limits.max_source_bytes = 4;
        let limited = TextEngine::with_config(markup).unwrap();
        let mut config = measure(&text, &font);
        config.syntax_mode = TextSyntaxMode::Plain;
        assert!(limited.measure_bounds_with_plain_fallback(&config).is_err());
        assert!(limited.measure_bounds_with_limit(&config, 2.0).is_err());
        assert!(limited.shape_line(&config).is_err());
        assert!(limited
            .rasterize_with_plain_fallback(&raster(&text, &font), 1.0, &HashMap::<_, ()>::new())
            .is_err());
        assert!(limited
            .extract_paths_with_plain_fallback(&paths(&text, &font))
            .is_err());
        assert!(limited
            .extract_pdf_with_plain_fallback(&pdf_config(&paths(&text, &font)))
            .is_err());

        let strict = TextEngine::with_font_resolution(&crate::FontResolutionOptions {
            load_system_fonts: false,
            missing_font: crate::MissingFontPolicy::Error,
            ..crate::default_font_resolution()
        })
        .unwrap();
        let missing = "UnavailableRegressionFont123".to_string();
        assert!(matches!(
            strict.measure_bounds_with_plain_fallback(&measure(&text, &missing)),
            Err(AvengerTextError::Typesetting(
                avenger_typst_label::LabelError::MissingFont { .. }
            ))
        ));
        assert!(strict
            .rasterize_with_plain_fallback(&raster(&text, &missing), 1.0, &HashMap::<_, ()>::new())
            .is_err());
        assert!(strict
            .extract_paths_with_plain_fallback(&paths(&text, &missing))
            .is_err());
        assert!(strict
            .extract_pdf_with_plain_fallback(&pdf_config(&paths(&text, &missing)))
            .is_err());
    }

    #[test]
    fn font_metrics_follow_resolved_face_and_scale() {
        let engine = engine();
        let config = FontMetricsConfig {
            font: "Lato",
            font_size: 16.0,
            font_weight: WEIGHT,
            font_style: STYLE,
        };
        let lato = engine.font_metrics(&config).unwrap();
        let mono = engine
            .font_metrics(&FontMetricsConfig {
                font: "DejaVu Sans Mono",
                ..config.clone()
            })
            .unwrap();
        assert_ne!(lato.height, mono.height);
        let large = engine
            .font_metrics(&FontMetricsConfig {
                font_size: 32.0,
                ..config
            })
            .unwrap();
        assert_eq!(large.height, lato.height * 2.0);
        assert_eq!(large.line_gap, lato.line_gap * 2.0);
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
