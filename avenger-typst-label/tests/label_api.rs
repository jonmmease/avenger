mod common;

use avenger_typst_label::{
    Color, LabelEngine, LabelError, LabelFrameItem, LabelOptions, LabelParamValue, LineCap,
    LineJoin, PdfOptions, Stroke, SvgOptions, TextItemKind, escape_text, pdf_items, svg_items,
};
use indexmap::IndexMap;

#[cfg(feature = "raster")]
use avenger_typst_label::{RasterOptions, rasterize};

use avenger_format::{DateTimeFormatBinding, NumberFormatBinding};
use avenger_format_datetime_d3::{D3DateTimeFormatConfig, D3DateTimeFormatProvider};
use avenger_format_number_d3::{D3NumberFormatConfig, D3NumberFormatProvider};
use avenger_typst_label::LabelFormatting;

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options())
        .unwrap()
        .with_number_formatting(NumberFormatBinding::new(
            D3NumberFormatProvider,
            D3NumberFormatConfig::new(),
        ))
        .with_datetime_formatting(DateTimeFormatBinding::new(
            D3DateTimeFormatProvider,
            D3DateTimeFormatConfig::new(),
        ))
}

fn assert_same_literal_rendering(text: &str) {
    let engine = engine();
    let options = LabelOptions::default();
    let literal = engine.compile_text(text, &options).unwrap();
    let escaped = engine.compile(&escape_text(text), &options).unwrap();

    assert_metrics_close(literal.metrics.width, escaped.metrics.width);
    assert_metrics_close(literal.metrics.height, escaped.metrics.height);
    assert_metrics_close(literal.metrics.baseline, escaped.metrics.baseline);
    assert_eq!(literal.semantic_text(), escaped.semantic_text());
    assert!(!literal.flags.has_math);
}

fn assert_metrics_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.001,
        "expected {actual} to be within 0.001 of {expected}"
    );
}

fn first_stroke(label: &avenger_typst_label::CompiledLabel) -> &Stroke {
    label
        .frame
        .items
        .iter()
        .find_map(|(_, item)| match item {
            LabelFrameItem::Shape(shape) => shape.item.stroke.as_ref(),
            _ => None,
        })
        .expect("label should contain a stroked shape")
}

#[test]
fn compile_text_plain_ascii_matches_escaped_compile() {
    assert_same_literal_rendering("Revenue by region");
}

#[test]
fn compile_text_literal_dollar_hash_brackets_matches_escaped_compile() {
    assert_same_literal_rendering("cost $5 #not-markup [brackets]");
}

#[test]
fn compile_text_typst_markup_punctuation_matches_escaped_compile() {
    for text in [
        "*literal* _literal_ `literal`",
        "a~b a-b a...b",
        "user@host <tag> /path [brackets]",
        "- item + item = value :colon",
        "\"quote\" 'quote' http://example.com #hash $dollar",
    ] {
        assert_same_literal_rendering(text);
    }
}

#[test]
fn compile_markup_resolves_default_smartquotes() {
    let engine = engine();
    let options = LabelOptions::default();

    let cases = [
        ("\"hello\"", "“hello”"),
        ("'hello'", "‘hello’"),
        ("5'", "5′"),
        ("5\"", "5″"),
        ("\"She said 'hi'\"", "“She said ‘hi’”"),
        ("\"a #emph[b]\"", "“a b”"),
        ("$x$'", "x’"),
        ("\\\"hello\\\"", "\"hello\""),
    ];

    for (source, expected) in cases {
        let label = engine
            .compile(source, &options)
            .unwrap_or_else(|err| panic!("{source} should compile, got {err:?}"));
        assert_eq!(label.semantic_text(), expected, "{source}");
    }
}

#[test]
fn compile_text_keeps_literal_straight_quotes() {
    let label = engine()
        .compile_text("\"hello\" and 'hello'", &LabelOptions::default())
        .unwrap();

    assert_eq!(label.semantic_text(), "\"hello\" and 'hello'");
}

#[test]
fn compile_text_unicode_emoji_bidi_complex_script_matches_escaped_compile() {
    assert_same_literal_rendering("Revenue 🚀 שלום नमस्ते");
}

#[test]
fn compile_text_exposes_source_relative_glyph_ranges_and_run_direction() {
    for source in ["abc אבג xyz", "ab中"] {
        let label = engine()
            .compile_text(source, &LabelOptions::default())
            .unwrap();
        let runs = label
            .frame
            .items
            .iter()
            .filter_map(|(_, item)| match item {
                LabelFrameItem::Text(text) if text.kind == TextItemKind::Plain => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>();

        if source.contains('א') {
            assert!(runs.iter().any(|run| run.is_rtl));
            assert!(runs.iter().any(|run| !run.is_rtl));
        }
        for run in runs {
            assert_eq!(&source[run.byte_range.clone()], run.text);
            for glyph in &run.glyphs {
                assert!(run.byte_range.start <= glyph.text_range.start);
                assert!(glyph.text_range.end <= run.byte_range.end);
                assert!(source.is_char_boundary(glyph.text_range.start));
                assert!(source.is_char_boundary(glyph.text_range.end));
            }
        }
    }
}

#[test]
fn compile_markup_style_wrappers_handle_emoji_bidi_and_complex_script() {
    let samples = [
        "#underline[Revenue 🚀 שלום नमस्ते]",
        "#emph[Revenue 🚀 שלום नमस्ते]",
        "#strong[Revenue 🚀 שלום नमस्ते]",
        "#upper[Revenue 🚀 שלום नमस्ते]",
    ];

    for source in samples {
        let label = engine()
            .compile(source, &LabelOptions::default())
            .unwrap_or_else(|err| panic!("{source} should compile, got {err:?}"));

        assert!(label.metrics.width > 0.0, "{source}");
        assert!(label.metrics.height > 0.0, "{source}");
        assert!(label.flags.has_markup, "{source}");
        assert!(label.semantic_text().contains('🚀'), "{source}");
        assert!(label.semantic_text().contains("שלום"), "{source}");
        assert!(label.semantic_text().contains("नमस्ते"), "{source}");
    }
}

#[test]
fn compile_resolves_named_emoji_and_symbol_aliases() {
    let label = engine()
        .compile(
            "Trend #emoji.chart.up #sym.arrow.r target",
            &LabelOptions::default(),
        )
        .unwrap();

    assert_eq!(label.semantic_text(), "Trend 📈 → target");
    assert!(label.metrics.width > 0.0);
    assert!(label.flags.has_markup);
}

#[test]
fn compile_unmatched_dollar_errors() {
    let err = engine()
        .compile("cost $5", &LabelOptions::default())
        .unwrap_err();
    assert!(matches!(err, LabelError::Syntax { .. }));
}

#[test]
fn compile_text_unmatched_dollar_succeeds() {
    let label = engine()
        .compile_text("cost $5", &LabelOptions::default())
        .unwrap();
    assert_eq!(label.semantic_text(), "cost $5");
    assert!(!label.flags.has_math);
}

#[test]
fn compile_resolves_text_params() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "series_name".to_string(),
        LabelParamValue::Str("Revenue".to_string()),
    );
    options
        .params
        .insert("threshold".to_string(), LabelParamValue::Float(2.5));
    options
        .params
        .insert("active".to_string(), LabelParamValue::Bool(true));

    let label = engine()
        .compile("#series_name >= #threshold (#active)", &options)
        .unwrap();

    assert_eq!(label.semantic_text(), "Revenue >= 2.5 (true)");
    assert!(label.flags.has_markup);
    assert!(!label.flags.has_math);
}

#[test]
fn compile_resolves_text_params_inside_static_markup() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "series_name".to_string(),
        LabelParamValue::Str("revenue".to_string()),
    );

    let label = engine().compile("#upper[#series_name]", &options).unwrap();

    assert_eq!(label.semantic_text(), "REVENUE");
    assert!(label.flags.has_markup);
}

#[test]
fn compile_lower_uses_context_sensitive_unicode_casing() {
    let label = engine()
        .compile("#lower[ΟΣ]", &LabelOptions::default())
        .unwrap();

    assert_eq!(label.semantic_text(), "ος");
    assert!(label.flags.has_markup);
}

#[test]
fn compile_text_treats_param_syntax_as_literal_text() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "series_name".to_string(),
        LabelParamValue::Str("Revenue".to_string()),
    );

    let label = engine().compile_text("#series_name", &options).unwrap();

    assert_eq!(label.semantic_text(), "#series_name");
    assert!(!label.flags.has_markup);
}

#[test]
fn compile_errors_for_unknown_text_param() {
    let err = engine()
        .compile("#series_name", &LabelOptions::default())
        .unwrap_err();

    assert_eq!(
        err,
        LabelError::UnsupportedSyntax {
            position: 0,
            message: "unknown label parameter"
        }
    );
}

#[test]
fn compile_numfmt_uses_number_locale_context() {
    let number = NumberFormatBinding::new(
        D3NumberFormatProvider,
        D3NumberFormatConfig::new()
            .with_locale("de-DE")
            .with_custom_locale(
                "de-DE",
                serde_json::from_str(include_str!(
                    "../../avenger-format-number-d3/tests/fixtures/locales/de-DE.json"
                ))
                .unwrap(),
            ),
    );
    let mut options = LabelOptions::default();
    options
        .params
        .insert("value".to_string(), LabelParamValue::Float(1234.5));

    let label = engine()
        .compile_with_formatting(
            "#numfmt(value, \",.1f\")",
            &options,
            LabelFormatting {
                number: Some(&number),
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(label.semantic_text(), "1.234,5");
}

#[test]
fn compile_datefmt_formats_temporal_param() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "report_date".to_string(),
        LabelParamValue::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()),
    );

    let label = engine()
        .compile("Report #datefmt(report_date, \"%B %-d, %Y\")", &options)
        .unwrap();

    assert_eq!(label.semantic_text(), "Report January 5, 2024");
}

#[test]
fn compile_errors_for_non_scalar_text_param() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "items".to_string(),
        LabelParamValue::Array(vec![LabelParamValue::Int(1)]),
    );

    let err = engine().compile("#items", &options).unwrap_err();

    assert_eq!(
        err,
        LabelError::UnsupportedSyntax {
            position: 0,
            message: "label parameter value cannot be rendered as text"
        }
    );
}

#[test]
fn compile_resolves_math_params() {
    let mut options = LabelOptions::default();
    options
        .params
        .insert("slope".to_string(), LabelParamValue::Float(2.5));
    options
        .params
        .insert("intercept".to_string(), LabelParamValue::Int(7));

    let label = engine()
        .compile("$y = #slope x + #intercept$", &options)
        .unwrap();

    assert!(label.metrics.width > 0.0);
    assert!(label.metrics.height > 0.0);
    assert!(label.flags.has_math);
    assert!(label.flags.has_markup);
}

#[test]
fn compile_errors_for_non_scalar_math_param() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "items".to_string(),
        LabelParamValue::Array(vec![LabelParamValue::Int(1)]),
    );

    let err = engine().compile("$#items$", &options).unwrap_err();

    assert_eq!(
        err,
        LabelError::UnsupportedSyntax {
            position: 1,
            message: "label parameter value cannot be rendered as math"
        }
    );
}

#[test]
fn math_names_stay_in_math_namespace_when_params_exist() {
    let engine = engine();
    let source = "$alpha + frac(1, 2) + sqrt(x) + bold(x)$";
    let baseline = engine.compile(source, &LabelOptions::default()).unwrap();

    let mut options = LabelOptions::default();
    options.params.insert(
        "series_name".to_string(),
        LabelParamValue::Str("param".to_string()),
    );

    let with_params = engine.compile(source, &options).unwrap();

    assert_metrics_close(with_params.metrics.width, baseline.metrics.width);
    assert_metrics_close(with_params.metrics.height, baseline.metrics.height);
    assert_metrics_close(with_params.metrics.baseline, baseline.metrics.baseline);
}

#[test]
fn compile_rejects_param_names_colliding_with_text_names() {
    for name in ["upper", "emoji", "sym", "red"] {
        let mut options = LabelOptions::default();
        options
            .params
            .insert(name.to_string(), LabelParamValue::Str("param".to_string()));

        let err = engine().compile("label", &options).unwrap_err();

        assert_eq!(
            err,
            LabelError::ParameterNameCollision {
                name: name.to_string(),
                namespace: "text"
            }
        );
    }
}

#[test]
fn compile_rejects_param_names_colliding_with_math_names() {
    for name in ["frac", "thin", "dif"] {
        let mut options = LabelOptions::default();
        options
            .params
            .insert(name.to_string(), LabelParamValue::Str("param".to_string()));

        let err = engine().compile("$x$", &options).unwrap_err();

        assert_eq!(
            err,
            LabelError::ParameterNameCollision {
                name: name.to_string(),
                namespace: "math"
            }
        );
    }
}

#[test]
fn compile_text_allows_param_names_colliding_with_markup_names() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "upper".to_string(),
        LabelParamValue::Str("param".to_string()),
    );
    options.params.insert(
        "frac".to_string(),
        LabelParamValue::Str("param".to_string()),
    );

    let label = engine().compile_text("#upper and frac", &options).unwrap();

    assert_eq!(label.semantic_text(), "#upper and frac");
    assert!(!label.flags.has_markup);
}

#[test]
fn compile_resolves_stroke_paint_param() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "series_color".to_string(),
        LabelParamValue::Str("red".to_string()),
    );

    let label = engine()
        .compile("#underline(stroke: series_color)[Series]", &options)
        .unwrap();
    let stroke = first_stroke(&label);

    assert_eq!(
        stroke.color,
        Color::rgba(1.0, 65.0 / 255.0, 54.0 / 255.0, 1.0)
    );
    assert!(stroke.width > 0.0);
}

#[test]
fn compile_resolves_stroke_string_param() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "series_stroke".to_string(),
        LabelParamValue::Str("1.5pt + blue".to_string()),
    );

    let label = engine()
        .compile("#underline(stroke: series_stroke)[Series]", &options)
        .unwrap();
    let stroke = first_stroke(&label);

    assert_eq!(
        stroke.color,
        Color::rgba(0.0, 116.0 / 255.0, 217.0 / 255.0, 1.0)
    );
    assert_metrics_close(stroke.width, 1.5);
}

#[test]
fn compile_resolves_stroke_dict_param() {
    let mut stroke_param = IndexMap::new();
    stroke_param.insert(
        "paint".to_string(),
        LabelParamValue::Str("maroon".to_string()),
    );
    stroke_param.insert(
        "thickness".to_string(),
        LabelParamValue::Str("2pt".to_string()),
    );
    stroke_param.insert("cap".to_string(), LabelParamValue::Str("round".to_string()));
    stroke_param.insert(
        "join".to_string(),
        LabelParamValue::Str("bevel".to_string()),
    );
    stroke_param.insert(
        "dash".to_string(),
        LabelParamValue::Str("dashed".to_string()),
    );
    stroke_param.insert("miter-limit".to_string(), LabelParamValue::Float(2.0));

    let mut options = LabelOptions::default();
    options.params.insert(
        "series_stroke".to_string(),
        LabelParamValue::Dict(stroke_param),
    );

    let label = engine()
        .compile("#underline(stroke: series_stroke)[Series]", &options)
        .unwrap();
    let stroke = first_stroke(&label);

    assert_eq!(
        stroke.color,
        Color::rgba(133.0 / 255.0, 20.0 / 255.0, 75.0 / 255.0, 1.0)
    );
    assert_metrics_close(stroke.width, 2.0);
    assert_eq!(stroke.line_cap, LineCap::Round);
    assert_eq!(stroke.line_join, LineJoin::Bevel);
    let dash = stroke.dash.as_ref().expect("dash should resolve");
    assert_eq!(dash.array.as_slice(), [3.0, 3.0].as_slice());
    assert_eq!(dash.phase, 0.0);
    assert_eq!(stroke.miter_limit, 2.0);
}

#[test]
fn compile_mixed_label_returns_ordered_frame_items() {
    let label = engine()
        .compile("Price \\$7, ratio $a / b$ = 0.94", &LabelOptions::default())
        .unwrap();

    assert!(label.metrics.width > 0.0);
    assert!(label.metrics.height > 0.0);
    assert!(label.flags.has_math);
    assert!(label.flags.has_markup);
    assert!(
        label
            .frame
            .items
            .iter()
            .any(|(_, item)| matches!(item, LabelFrameItem::Text(_)))
    );
    assert!(
        label
            .frame
            .items
            .iter()
            .any(|(_, item)| matches!(item, LabelFrameItem::Shape(_)))
    );
}

#[test]
fn svg_and_pdf_lowerers_consume_compiled_label() {
    let label = engine()
        .compile(
            "Price \\$7, ratio $frac(a, b)$ = 0.94",
            &LabelOptions::default(),
        )
        .unwrap();

    let svg = svg_items(&label, &SvgOptions::default()).unwrap();
    assert!(svg.items.iter().any(
        |(_, item)| matches!(item, LabelFrameItem::Text(text) if text.kind == TextItemKind::Plain)
    ));
    assert!(svg.items.iter().any(
        |(_, item)| matches!(item, LabelFrameItem::Text(text) if text.kind == TextItemKind::Math)
    ));
    assert!(
        svg.items
            .iter()
            .any(|(_, item)| matches!(item, LabelFrameItem::Shape(_)))
    );

    let pdf = pdf_items(&label, &PdfOptions::default()).unwrap();
    assert!(!pdf.glyph_runs.is_empty());
    assert!(!pdf.path_items.is_empty());
    assert!(!pdf.draw_items.is_empty());
    assert!(!pdf.font_resources.is_empty());
}

#[test]
#[cfg(feature = "raster")]
fn raster_lowerer_consumes_compiled_label() {
    let label = engine().compile("$R^2$", &LabelOptions::default()).unwrap();
    let raster = rasterize(&label, &RasterOptions { scale: 2.0 }).unwrap();

    assert_eq!(raster.scale, 2.0);
    assert!(raster.image.width > 0);
    assert!(raster.image.height > 0);
}

#[test]
fn datefmt_reports_value_errors_through_label_compilation() {
    let engine = engine().with_datetime_formatting(DateTimeFormatBinding::new(
        D3DateTimeFormatProvider,
        D3DateTimeFormatConfig::new().with_timezone("Asia/Tokyo"),
    ));
    let mut options = LabelOptions::default();
    let leap = chrono::NaiveDate::from_ymd_opt(2016, 12, 31)
        .unwrap()
        .and_hms_milli_opt(23, 59, 59, 1500)
        .unwrap();
    for (value, expected) in [
        (
            LabelParamValue::UtcDateTime(chrono::DateTime::<chrono::Utc>::MAX_UTC),
            "calendar range",
        ),
        (LabelParamValue::DateTime(leap), "leap seconds"),
    ] {
        options.params.insert("value".into(), value);
        assert!(matches!(
            engine.compile("#datefmt(value, \"%Y\")", &options),
            Err(LabelError::Engine { message, .. }) if message.contains(expected)
        ));
    }
}

#[test]
fn datetime_cache_tracks_request_configuration_and_input_type() {
    let engine = engine();
    let config = D3DateTimeFormatConfig::new().with_custom_locale(
        "fr-FR",
        serde_json::from_str(include_str!(
            "../../avenger-format-datetime-d3/tests/fixtures/locales/fr-FR.json"
        ))
        .unwrap(),
    );
    let mut options = LabelOptions::default();
    options.params.insert(
        "value".into(),
        LabelParamValue::UtcDateTime(
            chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .to_utc(),
        ),
    );
    for (pattern, locale, timezone, expected) in [
        ("%B %d %H:%M", "en-US", "UTC", "January 01 00:00"),
        ("%B %d %H:%M", "fr-FR", "UTC", "janvier 01 00:00"),
        (
            "%B %d %H:%M",
            "fr-FR",
            "America/New_York",
            "décembre 31 19:00",
        ),
        ("%Y %Z", "fr-FR", "America/New_York", "2023 -0500"),
    ] {
        options
            .params
            .insert("pattern".into(), LabelParamValue::Str(pattern.into()));
        let datetime = DateTimeFormatBinding::new(
            D3DateTimeFormatProvider,
            config.clone().with_locale(locale).with_timezone(timezone),
        );
        assert_eq!(
            engine
                .compile_with_formatting(
                    "#datefmt(value, pattern)",
                    &options,
                    LabelFormatting {
                        datetime: Some(&datetime),
                        ..Default::default()
                    }
                )
                .unwrap()
                .semantic_text(),
            expected
        );
    }
    options
        .params
        .insert("pattern".into(), LabelParamValue::Str("%Y".into()));
    let datetime = DateTimeFormatBinding::new(
        D3DateTimeFormatProvider,
        config.with_timezone("America/New_York"),
    );
    let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    for (value, expected) in [
        (LabelParamValue::Date(date), "2024"),
        (
            LabelParamValue::UtcDateTime(date.and_hms_opt(0, 0, 0).unwrap().and_utc()),
            "2023",
        ),
        (
            LabelParamValue::DateTime(date.and_hms_opt(0, 0, 0).unwrap()),
            "2024",
        ),
    ] {
        options.params.insert("value".into(), value);
        assert_eq!(
            engine
                .compile_with_formatting(
                    "#datefmt(value, pattern)",
                    &options,
                    LabelFormatting {
                        datetime: Some(&datetime),
                        ..Default::default()
                    }
                )
                .unwrap()
                .semantic_text(),
            expected
        );
    }
}

#[test]
fn numfmt_uses_typed_providers_and_reuses_preparation() {
    use avenger_format::{
        FormattedNumber, NumberFormatError, NumberFormatProvider, PreparedNumberFormatter,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[derive(Debug)]
    struct Provider(Arc<AtomicUsize>);
    struct Config {
        unit: String,
    }
    #[derive(Debug)]
    struct Prepared(String);
    impl NumberFormatProvider for Provider {
        type Config = Config;
        fn prepare(
            &self,
            config: &Config,
            pattern: &str,
        ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(Prepared(format!("{pattern} {}", config.unit))))
        }
    }
    impl PreparedNumberFormatter for Prepared {
        fn format(&self, value: f64) -> FormattedNumber {
            FormattedNumber::plain(format!("{}: {value}", self.0))
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let binding = NumberFormatBinding::new(
        Provider(calls.clone()),
        Config {
            unit: "items".into(),
        },
    );
    let first = engine().with_number_formatting(binding.clone());
    let mut options = LabelOptions::default();
    options.params.insert(
        "pattern".into(),
        LabelParamValue::Str("custom syntax".into()),
    );
    let source = "#numfmt(2, pattern)";
    for engine in [&first, &first.clone().with_number_formatting(binding)] {
        assert_eq!(
            engine.compile(source, &options).unwrap().semantic_text(),
            "custom syntax items: 2"
        );
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let other = NumberFormatBinding::new(
        Provider(calls.clone()),
        Config {
            unit: "widgets".into(),
        },
    );
    assert_eq!(
        first
            .compile_with_formatting(
                source,
                &options,
                LabelFormatting {
                    number: Some(&other),
                    ..Default::default()
                }
            )
            .unwrap()
            .semantic_text(),
        "custom syntax widgets: 2"
    );
    options
        .params
        .insert("pattern".into(), LabelParamValue::Str("updated".into()));
    assert_eq!(
        first.compile(source, &options).unwrap().semantic_text(),
        "updated items: 2"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[test]
fn numeric_markup_requires_explicit_formatting_but_plain_text_does_not() {
    let engine = LabelEngine::new(common::engine_options()).unwrap();
    let options = LabelOptions::default();
    assert_eq!(
        engine.compile("Revenue", &options).unwrap().semantic_text(),
        "Revenue"
    );
    assert!(
        engine
            .compile("#numfmt(42)", &options)
            .unwrap_err()
            .to_string()
            .contains("number formatting is not configured")
    );
}

#[test]
fn temporal_markup_requires_explicit_provider_selection() {
    let engine = LabelEngine::new(common::engine_options()).unwrap();
    let options = LabelOptions {
        params: [(
            "value".into(),
            LabelParamValue::UtcDateTime(chrono::DateTime::UNIX_EPOCH),
        )]
        .into(),
        ..Default::default()
    };
    assert_eq!(
        engine.compile("Date", &options).unwrap().semantic_text(),
        "Date"
    );
    assert!(
        engine
            .compile("#datefmt(value, \"%Y\")", &options)
            .unwrap_err()
            .to_string()
            .contains("datetime formatting is not configured")
    );
}
