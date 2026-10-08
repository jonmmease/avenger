use std::sync::{Arc, OnceLock};

use avenger_typst_label::{EngineOptions, LabelEngine};

use crate::{
    cache::{LabelKey, Memo},
    error::AvengerTextError,
    measurement::{FontMetrics, FontMetricsConfig, TextBounds},
    path::TextPathBuffer,
    pdf::TextPdfBuffer,
    rasterization::{TextRaster, TextRasterKey},
    types::{TextConfig, TextSyntaxMode},
    typeset::{bounds_from_metrics, typeset, typst_font_style, typst_font_weight, LabelSettings},
    DateTimeFormatProvider, FontOptions, NumberFormatProvider,
};

/// How many label measurements and rasters an engine remembers. Measurements are small, and
/// interactive charts re-measure the same few hundred labels from frame to frame.
const MEASUREMENT_MEMO_CAPACITY: usize = 8192;
const RASTER_MEMO_CAPACITY: usize = 1024;

/// Lays out labels with the label crate, and draws them as images, paths or PDF glyph runs.
/// Clones share one label engine and its memos of measurements and rasters. Setting a formatting
/// provider gives the engine new memos, since its labels may then read differently.
#[derive(Debug, Clone)]
pub struct TextEngine {
    typst: LabelEngine,
    settings: LabelSettings,
    measurements: Memo<LabelKey, TextBounds>,
    rasters: Memo<TextRasterKey, TextRaster>,
}

impl TextEngine {
    /// An engine with these fonts.
    pub fn new(fonts: &FontOptions) -> Self {
        Self::with_settings(fonts, LabelSettings::default())
    }

    pub(crate) fn with_settings(fonts: &FontOptions, settings: LabelSettings) -> Self {
        Self {
            typst: LabelEngine::new(EngineOptions {
                fonts: fonts.clone(),
            }),
            settings,
            measurements: Memo::new(MEASUREMENT_MEMO_CAPACITY),
            rasters: Memo::new(RASTER_MEMO_CAPACITY),
        }
    }

    /// Sets the provider of `#numfmt`, for every label.
    pub fn with_number_formatting(mut self, provider: Arc<dyn NumberFormatProvider>) -> Self {
        self.typst = self.typst.with_number_formatting(provider);
        self.with_new_memos()
    }

    /// The provider of `#numfmt`.
    pub fn number_format(&self) -> Option<&Arc<dyn NumberFormatProvider>> {
        self.typst.number_format()
    }

    /// Sets the provider of `#datetimefmt`, for every label.
    pub fn with_datetime_formatting(mut self, provider: Arc<dyn DateTimeFormatProvider>) -> Self {
        self.typst = self.typst.with_datetime_formatting(provider);
        self.with_new_memos()
    }

    /// The provider of `#datetimefmt`.
    pub fn datetime_format(&self) -> Option<&Arc<dyn DateTimeFormatProvider>> {
        self.typst.datetime_format()
    }

    fn with_new_memos(mut self) -> Self {
        self.measurements = Memo::new(MEASUREMENT_MEMO_CAPACITY);
        self.rasters = Memo::new(RASTER_MEMO_CAPACITY);
        self
    }

    pub fn measure_bounds(&self, config: &TextConfig) -> Result<TextBounds, AvengerTextError> {
        self.measurements
            .get_or_try_insert(LabelKey::new(config), || {
                let label = typeset(&self.typst, &self.settings, config)?;
                Ok(bounds_from_metrics(&label.metrics, config.font_size))
            })
    }

    pub fn measure_bounds_with_plain_fallback(
        &self,
        config: &TextConfig,
    ) -> Result<TextBounds, AvengerTextError> {
        plain_fallback(config, |config| self.measure_bounds(config))
    }

    pub fn measure_bounds_with_plain_fallback_or_approx(&self, config: &TextConfig) -> TextBounds {
        self.measure_bounds_with_plain_fallback(config)
            .unwrap_or_else(|_| approximate_text_bounds(config))
    }

    /// The metrics of the face that a text style uses first. `FontMetrics::fallback` gives
    /// approximate ones.
    pub fn font_metrics(
        &self,
        config: &FontMetricsConfig,
    ) -> Result<FontMetrics, AvengerTextError> {
        let metrics = self.typst.font_metrics(&avenger_typst_label::TextStyle {
            font_family: config.font.to_string(),
            font_size: config.font_size,
            font_weight: typst_font_weight(config.font_weight),
            font_style: typst_font_style(config.font_style),
            ..Default::default()
        })?;
        let height = metrics.ascent + metrics.descent;
        Ok(FontMetrics {
            ascent: metrics.ascent,
            descent: metrics.descent,
            height,
            line_gap: metrics.line_gap,
            line_height: height + metrics.line_gap,
        })
    }

    /// A label rasterized at a scale.
    pub fn rasterize(
        &self,
        config: &TextConfig,
        scale: f32,
    ) -> Result<TextRaster, AvengerTextError> {
        let key = TextRasterKey::new(LabelKey::new(config), config, scale);
        self.rasters.get_or_try_insert(key.clone(), || {
            crate::rasterization::rasterize(&self.typst, &self.settings, config, scale, key)
        })
    }

    pub fn rasterize_with_plain_fallback(
        &self,
        config: &TextConfig,
        scale: f32,
    ) -> Result<TextRaster, AvengerTextError> {
        plain_fallback(config, |config| self.rasterize(config, scale))
    }

    pub fn extract_paths(&self, config: &TextConfig) -> Result<TextPathBuffer, AvengerTextError> {
        crate::path::extract_paths(&self.typst, &self.settings, config)
    }

    pub fn extract_paths_with_plain_fallback(
        &self,
        config: &TextConfig,
    ) -> Result<TextPathBuffer, AvengerTextError> {
        plain_fallback(config, |config| self.extract_paths(config))
    }

    pub fn extract_pdf(&self, config: &TextConfig) -> Result<TextPdfBuffer, AvengerTextError> {
        crate::pdf::extract_pdf(&self.typst, &self.settings, config)
    }

    pub fn extract_pdf_with_plain_fallback(
        &self,
        config: &TextConfig,
    ) -> Result<TextPdfBuffer, AvengerTextError> {
        plain_fallback(config, |config| self.extract_pdf(config))
    }
}

impl Default for TextEngine {
    /// The bundled fonts and the system's.
    fn default() -> Self {
        Self::new(&crate::default_font_options())
    }
}

/// An output of a label, or, if the label's source is invalid, of its source read as plain text.
fn plain_fallback<T>(
    config: &TextConfig,
    output: impl Fn(&TextConfig) -> Result<T, AvengerTextError>,
) -> Result<T, AvengerTextError> {
    output(config).or_else(|error| {
        if !error.allows_plain_fallback() {
            return Err(error);
        }
        output(&TextConfig {
            syntax_mode: TextSyntaxMode::Plain,
            ..config.clone()
        })
    })
}

/// Bounds of one line of text, estimated from its length, within the layout's width.
fn approximate_text_bounds(config: &TextConfig) -> TextBounds {
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

/// An engine with the default fonts, which every caller shares.
pub fn default_text_engine() -> TextEngine {
    static DEFAULT_TEXT_ENGINE: OnceLock<TextEngine> = OnceLock::new();
    DEFAULT_TEXT_ENGINE.get_or_init(TextEngine::default).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        path::{TextPathItem, TextRun},
        pdf::TextPdfItem,
        types::{TextLayout, TextSyntaxMode},
        LabelParamValue, LabelParams,
    };

    fn engine() -> TextEngine {
        TextEngine::default().with_datetime_formatting(Arc::new(
            avenger_format_datetime_d3::D3DateTimeFormatProvider::new(),
        ))
    }

    /// A markup label at size 14.
    fn config<'a>(text: &'a str, font: &'a str) -> TextConfig<'a> {
        TextConfig {
            text,
            syntax_mode: TextSyntaxMode::TypstMarkup,
            font,
            font_size: 14.0,
            ..Default::default()
        }
    }

    /// A label's native text runs.
    fn runs(buffer: &TextPathBuffer) -> Vec<&TextRun> {
        buffer
            .items
            .iter()
            .filter_map(|item| match item {
                TextPathItem::Run(run) => Some(run),
                _ => None,
            })
            .collect()
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
        let configure = |engine: TextEngine, value: &str| {
            engine
                .with_number_formatting(Arc::new(Provider(value.to_owned())))
                .with_datetime_formatting(Arc::new(Provider(value.to_owned())))
        };
        let first = configure(engine(), "1");
        // Made from a clone of the first engine, whose memos the labels below fill first.
        let second = configure(first.clone(), "100000000");
        let params = LabelParams::from([(
            "value".into(),
            LabelParamValue::ZonedDateTime(chrono::DateTime::UNIX_EPOCH),
        )]);
        for source in ["#numfmt(42, \"custom\")", "#datetimefmt(value, \"custom\")"] {
            let mut label = config(source, "sans-serif");
            label.params = &params;
            let first_bounds = first.measure_bounds(&label).unwrap();
            let first_raster = first.rasterize(&label, 1.0).unwrap();
            let second_bounds = second.measure_bounds(&label).unwrap();
            assert!(second_bounds.width > first_bounds.width * 2.0);
            let second_raster = second.rasterize(&label, 1.0).unwrap();
            assert!(second_raster.image.is_some());
            assert_eq!(first_raster.bounds, first_bounds);
            assert_eq!(second_raster.bounds, second_bounds);
        }
    }

    #[test]
    fn top_level_engine_measures_mixed_math_and_named_emoji() {
        let engine = engine();
        let font = "sans-serif";
        let text = "Revenue #emoji.face $x^2$";

        let bounds = engine.measure_bounds(&config(text, font)).unwrap();
        assert!(bounds.width > 0.0);
        assert!(bounds.height >= 14.0);

        let buffer = engine.extract_paths(&config(text, font)).unwrap();
        assert!(buffer
            .items
            .iter()
            .any(|item| matches!(item, TextPathItem::Shape(_))));
        let runs = runs(&buffer);
        assert!(runs.iter().any(|run| run.text.contains("Revenue ")));
        assert!(runs.iter().any(|run| run.text.contains('😀')));
        assert!(runs.iter().all(|run| !run.text.contains("#emoji")));
    }

    #[test]
    fn top_level_engine_measures_bidi_and_complex_script_text() {
        let engine = engine();
        let font = "sans-serif";
        let samples = [("ABC שלום", "שלום"), ("नमस्ते data", "नमस्ते")];

        for (sample, expected_text) in samples {
            let text = sample;
            let bounds = engine.measure_bounds(&config(text, font)).unwrap();
            assert!(bounds.width > 0.0, "{sample} should have positive width");
            assert!(bounds.height >= 14.0, "{sample} should have line height");

            let buffer = engine.extract_paths(&config(text, font)).unwrap();
            assert!(
                runs(&buffer)
                    .iter()
                    .any(|run| run.text.contains(expected_text)),
                "{sample} should preserve native text content in path extraction"
            );
        }
    }

    #[test]
    fn top_level_engine_rasterizes_whole_line_with_math_and_emoji() {
        let raster = engine()
            .rasterize(&config("Hi #emoji.face $x$", "sans-serif"), 2.0)
            .unwrap();
        assert!(raster.bounds.width > 0.0);
        assert!(raster.image.is_some());
        #[cfg(target_os = "macos")]
        assert!(
            colored_pixel_count(raster.image.as_ref().unwrap().as_raw()) > 20,
            "emoji text raster should contain colored pixels on macOS"
        );
    }

    #[test]
    fn empty_labels_rasterize_to_no_image() {
        let raster = engine().rasterize(&TextConfig::default(), 1.0).unwrap();
        assert!(raster.image.is_none());
        assert_eq!((raster.bounds.width, raster.bounds.height), (0.0, 12.0));
    }

    #[test]
    fn parameter_updates_invalidate_measurement_and_raster_caches() {
        let font = "Lato";
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
            let text = source;
            let mut measurement = config(text, font);
            measurement.params = &params_a;
            let bounds_a = engine.measure_bounds(&measurement).unwrap();
            measurement.params = &params_b;
            let bounds_b = engine.measure_bounds(&measurement).unwrap();
            assert_ne!(bounds_a.width, bounds_b.width);
            assert_eq!(
                bounds_b,
                self::engine().measure_bounds(&measurement).unwrap()
            );

            let mut config = config(text, font);
            config.params = &params_a;
            let raster_a = engine.rasterize(&config, 2.0).unwrap();
            config.params = &params_b;
            let raster_b = engine.rasterize(&config, 2.0).unwrap();
            assert_ne!(raster_a.key, raster_b.key);
            assert_eq!(raster_b.bounds, bounds_b);
        }
    }

    #[test]
    fn multi_line_bounds_agree_across_outputs() {
        let engine = engine();
        let font = "Lato";
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
            let text = source;
            let label = TextConfig {
                syntax_mode,
                layout,
                ..config(text, font)
            };
            let bounds = engine.measure_bounds(&label).unwrap();
            // Several lines, which the memo keeps apart from one line of the same text.
            let one = TextConfig {
                layout: TextLayout::default(),
                syntax_mode: TextSyntaxMode::Plain,
                ..label.clone()
            };
            assert!(bounds.height > engine.measure_bounds(&one).unwrap().height + 10.0);
            let rasterized = engine.rasterize(&label, 2.0).unwrap();
            assert_eq!(rasterized.bounds, bounds, "{source}");
            assert_eq!(
                engine.extract_paths(&label).unwrap().bounds,
                bounds,
                "{source}"
            );
            let pdf = engine.extract_pdf(&label).unwrap();
            assert_eq!(pdf.bounds, bounds, "{source}");
        }
    }

    #[test]
    fn several_lines_anchor_their_first_baseline_and_align_inside_their_box() {
        let engine = engine();
        let font = "Lato";
        let text = "short\na much longer line";
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
            let paths = TextConfig {
                syntax_mode: TextSyntaxMode::PlainLines,
                layout,
                ..config(text, font)
            };
            let buffer = engine.extract_paths(&paths).unwrap();
            assert_eq!(buffer.bounds.width, 200.0);
            // The first run's baseline is the box's ascent below its top, which Alphabetic
            // anchors.
            let runs = runs(&buffer);
            let baseline = runs[0].baseline;
            assert!((baseline - buffer.bounds.ascent).abs() < 1e-3, "{baseline}");
            // Each line sits where its alignment puts it in the box, whatever the anchor.
            for run in runs {
                let free = 200.0 - run.width;
                assert!((run.x - free * offset).abs() < 0.5, "{align:?}: {run:?}");
            }
        }
    }

    #[test]
    fn line_heights_space_glyph_baselines_and_line_boxes_add_half_the_gap() {
        let engine = engine();
        let font = "Lato";
        let text = "a\nb\nc";
        let baselines = |layout| {
            let paths = TextConfig {
                syntax_mode: TextSyntaxMode::PlainLines,
                layout,
                ..config(text, font)
            };
            let pdf = engine.extract_pdf(&paths).unwrap();
            let mut ys: Vec<f32> = pdf
                .items
                .iter()
                .filter_map(|item| match item {
                    TextPdfItem::Glyphs(run) => Some(run.transform.apply(run.glyphs[0].position).y),
                    TextPdfItem::Shape(_) => None,
                })
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
        let font = "Lato";
        let height = |source: &str| {
            let text = source;
            engine.measure_bounds(&config(text, font)).unwrap().height
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
        let font = "Lato";
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
            let text = source;
            let params = series_name_params("An expanded parameter label");
            let mut measure = config(text, font);
            measure.params = &params;
            measure.layout = layout(35.0);
            let mut raster_config = config(text, font);
            raster_config.params = &params;
            raster_config.layout = layout(35.0);
            let mut path_config = config(text, font);
            path_config.params = &params;
            path_config.layout = layout(35.0);
            let expected = engine.measure_bounds(&measure).unwrap();
            assert!(expected.width <= 35.0, "{source}: {expected:?}");
            let raster = engine.rasterize(&raster_config, 2.0).unwrap();
            assert_eq!(raster.bounds, expected);
            // Within the width, but for the ellipsis's overhang.
            let image = raster.image.as_ref().unwrap();
            assert!(raster.x + image.width() as f32 / 2.0 <= 36.0, "{source}");
            let path = engine.extract_paths(&path_config).unwrap();
            assert_eq!(path.bounds, expected);
            let pdf = engine.extract_pdf(&path_config).unwrap();
            assert_eq!(pdf.bounds, expected);
            assert!(
                pdf.semantic_text.ends_with('…'),
                "{source}: {}",
                pdf.semantic_text
            );
            assert!(!pdf.semantic_text.contains("#series_name"));
            raster_config.layout = layout(25.0);
            let narrower = engine.rasterize(&raster_config, 2.0).unwrap();
            assert_ne!(raster.key, narrower.key);
        }
    }

    #[test]
    fn limit_and_font_errors_survive_the_plain_fallback() {
        let mut settings = LabelSettings::default();
        settings.limits.max_source_bytes = 4;
        let limited = TextEngine::with_settings(&crate::default_font_options(), settings);
        let plain = TextConfig {
            syntax_mode: TextSyntaxMode::Plain,
            ..config("eight!!!", "Lato")
        };
        assert!(limited.measure_bounds_with_plain_fallback(&plain).is_err());
        // Cutting the text to a width doesn't get it under the limit either.
        let cut = TextConfig {
            layout: TextLayout {
                width: crate::LabelWidth::Max(2.0),
                wrap: false,
                ellipsis: true,
                ..TextLayout::default()
            },
            ..plain.clone()
        };
        assert!(limited.measure_bounds(&cut).is_err());

        let strict = TextEngine::new(&crate::FontOptions {
            load_system_fonts: false,
            missing_font: crate::MissingFontPolicy::Error,
            ..crate::default_font_options()
        });
        assert!(matches!(
            strict.extract_paths_with_plain_fallback(&config(
                "eight!!!",
                "UnavailableRegressionFont123"
            )),
            Err(AvengerTextError::Typesetting(
                avenger_typst_label::LabelError::MissingFont { .. }
            ))
        ));
    }

    #[test]
    fn font_metrics_follow_resolved_face_and_scale() {
        let engine = engine();
        let config = FontMetricsConfig {
            font: "Lato",
            font_size: 16.0,
            font_weight: crate::types::FontWeight::default(),
            font_style: crate::types::FontStyle::Normal,
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
        let font = "sans-serif";
        let text = "before $x^$ after";

        assert!(engine.measure_bounds(&config(text, font)).is_err());
        assert!(engine.extract_paths(&config(text, font)).is_err());
        assert!(engine.rasterize(&config(text, font), 2.0).is_err());
    }

    #[test]
    fn top_level_engine_plain_fallback_displays_invalid_math_as_text() {
        let engine = engine();
        let font = "sans-serif";
        let text = "before $x^$ after";

        let bounds = engine
            .measure_bounds_with_plain_fallback(&config(text, font))
            .unwrap();
        assert!(bounds.width > 0.0);

        let buffer = engine
            .extract_paths_with_plain_fallback(&config(text, font))
            .unwrap();
        assert!(matches!(
            buffer.items.as_slice(),
            [TextPathItem::Run(run)] if run.text == text
        ));

        let raster = engine
            .rasterize_with_plain_fallback(&config(text, font), 2.0)
            .unwrap();
        assert!(raster.image.is_some());
    }
}
