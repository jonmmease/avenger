use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use crate::{
    error::AvengerTextError,
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
    layout: String,
    params: String,
    number_format: Option<crate::ProviderIdentity<dyn crate::NumberFormatProvider>>,
    datetime_format: Option<crate::ProviderIdentity<dyn crate::DateTimeFormatProvider>>,
}

impl MeasureBoundsCacheKey {
    fn new(
        config: &TextMeasurementConfig,
        number_format: Option<&std::sync::Arc<dyn crate::NumberFormatProvider>>,
        datetime_format: Option<&std::sync::Arc<dyn crate::DateTimeFormatProvider>>,
    ) -> Self {
        Self {
            text: config.text.to_string(),
            font: config.font.to_string(),
            font_size_bits: config.font_size.to_bits(),
            font_weight: format!("{:?}", config.font_weight),
            font_style: format!("{:?}", config.font_style),
            syntax_mode: config.syntax_mode,
            layout: format!("{:?}", config.layout),
            params: crate::label_params_fingerprint(config.params),
            number_format: config
                .number_format
                .or(number_format)
                .map(crate::ProviderIdentity::new),
            datetime_format: config
                .datetime_format
                .or(datetime_format)
                .map(crate::ProviderIdentity::new),
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

    pub fn with_config(math: TextMarkupConfig) -> Self {
        Self::with_config_and_fonts(math, &crate::fonts::default_font_options())
    }

    pub fn with_config_and_fonts(math: TextMarkupConfig, fonts: &crate::FontOptions) -> Self {
        let options = avenger_typst_label::EngineOptions {
            fonts: fonts.clone(),
        };
        Self::new(avenger_typst_label::LabelEngine::new(options), math)
    }

    pub fn with_default_config() -> Self {
        Self::with_config(TextMarkupConfig::default())
    }

    pub fn with_fonts(fonts: &crate::FontOptions) -> Self {
        Self::with_config_and_fonts(TextMarkupConfig::default(), fonts)
    }

    /// Set the provider used by numeric labels.
    pub fn with_number_formatting(
        mut self,
        provider: std::sync::Arc<dyn crate::NumberFormatProvider>,
    ) -> Self {
        self.typst = self.typst.with_number_formatting(provider);
        self
    }

    /// Provider inherited by labels without their own number settings.
    pub fn number_format(&self) -> Option<&std::sync::Arc<dyn crate::NumberFormatProvider>> {
        self.typst.number_format()
    }

    /// Set the provider used by temporal labels.
    pub fn with_datetime_formatting(
        mut self,
        provider: std::sync::Arc<dyn crate::DateTimeFormatProvider>,
    ) -> Self {
        self.typst = self.typst.with_datetime_formatting(provider);
        self
    }

    /// Provider inherited by labels without their own datetime settings.
    pub fn datetime_format(&self) -> Option<&std::sync::Arc<dyn crate::DateTimeFormatProvider>> {
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
            .unwrap_or_else(|_| approximate_text_bounds(config))
    }

    /// Read metrics from the resolved font face. Use `FontMetrics::fallback`
    /// explicitly when approximate proportions are acceptable.
    pub fn font_metrics(
        &self,
        config: &FontMetricsConfig,
    ) -> Result<FontMetrics, AvengerTextError> {
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

/// Bounds of one line of text, estimated from its length, within the layout's width.
fn approximate_text_bounds(config: &TextMeasurementConfig) -> TextBounds {
    let height = config.font_size.max(1.0);
    let ascent = height * 0.8;
    let width = config.text.chars().count() as f32 * height * 0.6;
    TextBounds {
        width: match config.layout.width {
            crate::LabelWidth::Auto => width,
            crate::LabelWidth::Max(max) => width.min(max),
            crate::LabelWidth::Fixed(fixed) => fixed,
        },
        height,
        ascent,
        descent: height - ascent,
        leading: height * 0.2,
    }
}

pub fn default_text_engine() -> TextEngine {
    static DEFAULT_TEXT_ENGINE: OnceLock<TextEngine> = OnceLock::new();
    DEFAULT_TEXT_ENGINE
        .get_or_init(TextEngine::with_default_config)
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        path::{TextPathExtractionConfig, TextPathKind},
        rasterization::TextRasterizationConfig,
        types::{FontStyle, FontWeight, FontWeightNameSpec, TextLayout, TextSyntaxMode},
        LabelParamValue, LabelParams,
    };

    static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
    static STYLE: FontStyle = FontStyle::Normal;
    static COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn engine() -> TextEngine {
        TextEngine::with_default_config().with_datetime_formatting(Arc::new(
            avenger_format_datetime_d3::D3DateTimeFormatProvider::new(),
        ))
    }

    fn measure<'a>(text: &'a String, font: &'a String) -> TextMeasurementConfig<'a> {
        TextMeasurementConfig {
            text,
            font,
            font_size: 14.0,
            font_weight: WEIGHT,
            font_style: STYLE,
            syntax_mode: TextSyntaxMode::TypstMarkup,
            layout: crate::types::TextLayout::default(),
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
            layout: crate::types::TextLayout::default(),
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
            layout: crate::types::TextLayout::default(),
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
            layout: config.layout,
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
    fn formatter_providers_invalidate_measurement_and_raster_caches() {
        use avenger_format::*;
        #[derive(Debug)]
        struct Provider(String);
        #[derive(Debug)]
        struct Prepared(String);
        impl NumberFormatProvider for Provider {
            fn prepare(
                &self,
                _: &str,
            ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
                Ok(Arc::new(Prepared(self.0.clone())))
            }
        }
        impl DateTimeFormatProvider for Provider {
            fn prepare_date(
                &self,
                _: &str,
            ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
                Ok(Arc::new(Prepared(self.0.clone())))
            }
            fn prepare_naive(
                &self,
                _: &str,
            ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
                Ok(Arc::new(Prepared(self.0.clone())))
            }
            fn prepare_zoned(
                &self,
                _: &str,
            ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
                Ok(Arc::new(Prepared(self.0.clone())))
            }
            fn default_calendar_patterns(&self) -> CalendarPatterns {
                unreachable!("labels format explicit patterns")
            }
        }
        impl PreparedNumberFormatter for Prepared {
            fn format(&self, _: f64) -> FormattedNumber {
                FormattedNumber::plain(self.0.clone())
            }
        }
        impl PreparedDateFormatter for Prepared {
            fn format(&self, _: chrono::NaiveDate) -> Result<String, DateTimeFormatError> {
                Ok(self.0.clone())
            }
        }
        impl PreparedNaiveDateTimeFormatter for Prepared {
            fn format(&self, _: chrono::NaiveDateTime) -> Result<String, DateTimeFormatError> {
                Ok(self.0.clone())
            }
        }
        impl PreparedZonedDateTimeFormatter for Prepared {
            fn format(
                &self,
                _: chrono::DateTime<chrono::Utc>,
            ) -> Result<String, DateTimeFormatError> {
                Ok(self.0.clone())
            }
            fn timezone(&self) -> chrono_tz::Tz {
                chrono_tz::UTC
            }
        }
        let configure = |value: &str| {
            engine()
                .with_number_formatting(Arc::new(Provider(value.to_owned())))
                .with_datetime_formatting(Arc::new(Provider(value.to_owned())))
        };
        let first = configure("1");
        let second = configure("100000000");
        let params = LabelParams::from([(
            "value".into(),
            LabelParamValue::ZonedDateTime(chrono::DateTime::UNIX_EPOCH),
        )]);
        let font = "sans-serif".to_string();
        for source in ["#numfmt(42, \"custom\")", "#datetimefmt(value, \"custom\")"] {
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
            .any(|item| matches!(item.kind, TextPathKind::Glyph { .. })));
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
    fn parameter_updates_invalidate_measurement_and_raster_caches() {
        use crate::rasterization::CachedTextRasterization;

        let font = "Lato".to_string();
        let mut cases = vec![(
            "#series_name",
            series_name_params("Revenue"),
            series_name_params("Cost"),
        )];
        let params = |month| {
            let value = chrono::NaiveDate::from_ymd_opt(2500, month, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            LabelParams::from([(
                "value".to_string(),
                LabelParamValue::ZonedDateTime(value.and_utc()),
            )])
        };
        cases.push((r#"#datetimefmt(value, "%B")"#, params(1), params(9)));
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

    /// Configs of every output for one label.
    fn configs<'a>(
        text: &'a String,
        font: &'a String,
        syntax_mode: TextSyntaxMode,
        layout: TextLayout,
    ) -> (
        TextMeasurementConfig<'a>,
        TextRasterizationConfig<'a>,
        TextPathExtractionConfig<'a>,
    ) {
        let mut measure = measure(text, font);
        measure.syntax_mode = syntax_mode;
        measure.layout = layout;
        let mut raster = raster(text, font);
        raster.syntax_mode = syntax_mode;
        raster.layout = layout;
        let mut paths = paths(text, font);
        paths.syntax_mode = syntax_mode;
        paths.layout = layout;
        (measure, raster, paths)
    }

    #[test]
    fn multi_line_bounds_agree_across_outputs() {
        let engine = engine();
        let font = "Lato".to_string();
        let wrapped = TextLayout {
            width: crate::LabelWidth::Max(80.0),
            ..TextLayout::default()
        };
        for (source, syntax_mode, layout) in [
            (
                "Revenue by region in millions of dollars",
                TextSyntaxMode::Plain,
                wrapped,
            ),
            (
                "Revenue\n(millions)\nby region",
                TextSyntaxMode::PlainLines,
                TextLayout::default(),
            ),
            (
                "Revenue \\ *ratio* $x^2$",
                TextSyntaxMode::TypstMarkup,
                TextLayout::default(),
            ),
        ] {
            let text = source.to_string();
            let (measure, raster, paths) = configs(&text, &font, syntax_mode, layout);
            let bounds = engine.measure_bounds(&measure).unwrap();
            // Several lines, which the memo keeps apart from one line of the same text.
            let one = TextMeasurementConfig {
                layout: TextLayout::default(),
                syntax_mode: TextSyntaxMode::Plain,
                ..measure.clone()
            };
            assert!(bounds.height > engine.measure_bounds(&one).unwrap().height + 10.0);
            let rasterized = engine
                .rasterize(&raster, 2.0, &HashMap::<_, ()>::new())
                .unwrap();
            assert_eq!(rasterized.text_bounds, bounds, "{source}");
            assert_eq!(
                engine.extract_paths(&paths).unwrap().bounds,
                bounds,
                "{source}"
            );
            let pdf = engine.extract_pdf(&pdf_config(&paths)).unwrap();
            assert_eq!(pdf.bounds, bounds, "{source}");
        }
    }

    #[test]
    fn several_lines_anchor_their_first_baseline_and_align_inside_their_box() {
        let engine = engine();
        let font = "Lato".to_string();
        let text = "short\na much longer line".to_string();
        for (align, offset) in [
            (crate::LabelAlign::Left, 0.0),
            (crate::LabelAlign::Center, 0.5),
            (crate::LabelAlign::Right, 1.0),
        ] {
            let layout = TextLayout {
                width: crate::LabelWidth::Fixed(200.0),
                align,
                ..TextLayout::default()
            };
            let (_, _, paths) = configs(&text, &font, TextSyntaxMode::PlainLines, layout);
            let buffer = engine.extract_paths(&paths).unwrap();
            assert_eq!(buffer.bounds.width, 200.0);
            // The first run's baseline is the box's ascent below its top, which Alphabetic
            // anchors.
            let first = &buffer.plain_runs[0];
            let baseline = first.baseline;
            assert!((baseline - buffer.bounds.ascent).abs() < 1e-3, "{baseline}");
            // Each line sits where its alignment puts it in the box, whatever the anchor.
            for run in &buffer.plain_runs {
                let free = 200.0 - run.width;
                assert!((run.x - free * offset).abs() < 0.5, "{align:?}: {run:?}");
            }
        }
    }

    #[test]
    fn line_heights_space_glyph_baselines_and_line_boxes_add_half_the_gap() {
        let engine = engine();
        let font = "Lato".to_string();
        let text = "a\nb\nc".to_string();
        let baselines = |layout| {
            let (_, _, paths) = configs(&text, &font, TextSyntaxMode::PlainLines, layout);
            let pdf = engine.extract_pdf(&pdf_config(&paths)).unwrap();
            let mut ys: Vec<f32> = pdf
                .glyph_runs
                .iter()
                .map(|run| run.transform.apply(run.glyphs[0].position).y)
                .collect();
            ys.sort_by(f32::total_cmp);
            (ys, pdf.bounds)
        };
        let (ys, bounds) = baselines(TextLayout {
            line_height: crate::LabelLineHeight::Fixed(30.0),
            ..TextLayout::default()
        });
        assert_eq!(ys.len(), 3);
        for pair in ys.windows(2) {
            assert!((pair[1] - pair[0] - 30.0).abs() < 1e-3, "{ys:?}");
        }
        // The gap between plain lines' boxes is the pitch less the font size.
        assert!((bounds.leading - 16.0).abs() < 1e-3, "{bounds:?}");
        // With Typst's spacing, plain lines leave a positive gap.
        let (_, auto) = baselines(TextLayout::default());
        assert!(auto.leading > 0.0 && auto.leading < 14.0, "{auto:?}");
    }

    #[test]
    fn math_and_plain_text_share_the_padding_rule() {
        let engine = engine();
        let font = "Lato".to_string();
        let height = |source: &str| {
            let text = source.to_string();
            engine
                .measure_bounds(&measure(&text, &font))
                .unwrap()
                .height
        };
        // A line shorter than the font size is padded to it, with math or not.
        assert_eq!(height("Radius"), 14.0);
        assert_eq!(height("$x$"), 14.0);
        // Taller content is as tall as it is.
        assert!(height("$display(sum_(i=1)^n x_i)$") > 20.0);
    }

    #[test]
    fn widths_cut_rich_text_and_share_bounds_across_outputs() {
        let engine = engine();
        let font = "Lato".to_string();
        let layout = |width| TextLayout {
            width: crate::LabelWidth::Max(width),
            wrap: false,
            ellipsis: true,
            ..TextLayout::default()
        };
        for source in [
            "#series_name",
            "*A long bold label* $x^2$",
            "#strong[A long label]",
        ] {
            let text = source.to_string();
            let params = series_name_params("An expanded parameter label");
            let mut measure = measure(&text, &font);
            measure.params = &params;
            measure.layout = layout(35.0);
            let mut raster_config = raster(&text, &font);
            raster_config.params = &params;
            raster_config.layout = layout(35.0);
            let mut path_config = paths(&text, &font);
            path_config.params = &params;
            path_config.layout = layout(35.0);
            let expected = engine.measure_bounds(&measure).unwrap();
            assert!(expected.width <= 35.0, "{source}: {expected:?}");
            let raster = engine
                .rasterize(&raster_config, 2.0, &HashMap::<_, ()>::new())
                .unwrap();
            assert_eq!(raster.text_bounds, expected);
            for (entry, position) in &raster.entries {
                assert_eq!(entry.cache_key.text, source);
                // Within the width, but for the ellipsis's overhang.
                assert!(
                    position.x + entry.bbox.width as f32 / 2.0 <= 36.0,
                    "{source}"
                );
            }
            let path = engine.extract_paths(&path_config).unwrap();
            assert_eq!(path.bounds, expected);
            let pdf = engine.extract_pdf(&pdf_config(&path_config)).unwrap();
            assert_eq!(pdf.bounds, expected);
            assert!(
                pdf.semantic_text.ends_with('…'),
                "{source}: {}",
                pdf.semantic_text
            );
            assert!(!pdf.semantic_text.contains("#series_name"));
            raster_config.layout = layout(25.0);
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
        let limited = TextEngine::with_config(markup);
        let mut config = measure(&text, &font);
        config.syntax_mode = TextSyntaxMode::Plain;
        assert!(limited.measure_bounds_with_plain_fallback(&config).is_err());
        let cut = TextMeasurementConfig {
            layout: TextLayout {
                width: crate::LabelWidth::Max(2.0),
                wrap: false,
                ellipsis: true,
                ..TextLayout::default()
            },
            ..config.clone()
        };
        assert!(limited.measure_bounds(&cut).is_err());
        assert!(limited
            .rasterize_with_plain_fallback(&raster(&text, &font), 1.0, &HashMap::<_, ()>::new())
            .is_err());
        assert!(limited
            .extract_paths_with_plain_fallback(&paths(&text, &font))
            .is_err());
        assert!(limited
            .extract_pdf_with_plain_fallback(&pdf_config(&paths(&text, &font)))
            .is_err());

        let strict = TextEngine::with_fonts(&crate::FontOptions {
            load_system_fonts: false,
            missing_font: crate::MissingFontPolicy::Error,
            ..crate::default_font_options()
        });
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
}
