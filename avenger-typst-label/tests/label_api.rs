mod common;

use avenger_typst_label::{
    CompiledLabel, FrameItem, LabelEngine, LabelError, LabelOptions, LabelParamValue,
    LineCap, LineJoin, PdfItem, PdfOptions, Stroke, SvgItem, SvgOptions, escape_text,
    pdf_items, svg_items,
};
use indexmap::IndexMap;

#[cfg(feature = "raster")]
use avenger_typst_label::{RasterOptions, rasterize};

use avenger_color::AbsoluteColor;
use avenger_format::{DateTimeFormatProvider, NumberFormatProvider};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_typst_label::LabelFormatting;
use chrono_tz::{America::New_York, Asia::Tokyo, UTC};
use std::sync::Arc;

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options())
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
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

/// The items of a label's frame at any depth.
fn items(label: &CompiledLabel) -> Vec<&FrameItem> {
    let mut items = vec![];
    label.frame.visit(Default::default(), &mut |_, item| items.push(item));
    items
}

fn first_stroke(label: &CompiledLabel) -> &Stroke {
    items(label)
        .into_iter()
        .find_map(|item| match item {
            FrameItem::Shape(shape) => shape.stroke.as_ref(),
            _ => None,
        })
        .expect("label should contain a stroked shape")
}

/// The message of a source's error.
fn error(
    source: &str,
    options: &LabelOptions,
) -> (String, std::ops::Range<usize>, Vec<String>) {
    match engine().compile(source, options).unwrap_err() {
        LabelError::Source { range, message, hints } => (message, range, hints),
        other => panic!("{source}: {other:?}"),
    }
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
fn compile_text_collapses_whitespace_like_markup() {
    // Runs of whitespace are one space, and line breaks are spaces (D2, D25).
    for text in ["a   b", "a\tb", "a\nb", "a\r\n\r\nb", "  a b  "] {
        assert_same_literal_rendering(text);
    }
    let engine = engine();
    let options = LabelOptions::default();
    assert_eq!(
        engine.compile_text("a \n\n b", &options).unwrap().metrics,
        engine.compile_text("a b", &options).unwrap().metrics
    );
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
        ("$x$'", "𝑥’"),
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
fn glyphs_map_to_source_and_runs_keep_visual_order() {
    for source in ["abc אבג xyz", "ab中"] {
        let label = engine().compile_text(source, &LabelOptions::default()).unwrap();
        for (_, text) in label.frame.text_items() {
            for glyph in &text.glyphs {
                assert!(text.source.start <= glyph.source.start);
                assert!(glyph.source.end <= text.source.end);
                assert!(source.is_char_boundary(glyph.source.start));
                assert!(source.is_char_boundary(glyph.source.end));
            }
        }
    }

    // Right-to-left glyphs come in visual order, so their sources run backwards.
    let source = "abc אבג xyz";
    let label = engine().compile_text(source, &LabelOptions::default()).unwrap();
    let hebrew: Vec<_> = label
        .frame
        .text_items()
        .into_iter()
        .flat_map(|(_, text)| text.glyphs.iter())
        .filter(|glyph| {
            source[glyph.source.clone()].chars().all(|c| ('א'..='ת').contains(&c))
        })
        .map(|glyph| glyph.source.start)
        .collect();
    assert_eq!(hebrew.len(), 3);
    assert!(hebrew.windows(2).all(|pair| pair[0] > pair[1]), "{hebrew:?}");
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
        assert!(label.semantic_text().contains('🚀'), "{source}");
        assert!(label.semantic_text().contains("שלום"), "{source}");
        assert!(label.semantic_text().contains("नमस्ते"), "{source}");
    }
}

#[test]
fn compile_resolves_named_emoji_and_symbol_aliases() {
    let label = engine()
        .compile("Trend #emoji.chart.up #sym.arrow.r target", &LabelOptions::default())
        .unwrap();

    assert_eq!(label.semantic_text(), "Trend 📈 → target");
    assert!(label.metrics.width > 0.0);
}

#[test]
fn compile_unmatched_dollar_errors() {
    let (message, range, _) = error("cost $5", &LabelOptions::default());
    assert_eq!((message.as_str(), range), ("unclosed delimiter", 5..6));
}

#[test]
fn compile_text_unmatched_dollar_succeeds() {
    let label = engine().compile_text("cost $5", &LabelOptions::default()).unwrap();
    assert_eq!(label.semantic_text(), "cost $5");
    assert!(!label.flags.has_math);
}

#[test]
fn compile_resolves_text_params() {
    let mut options = LabelOptions::default();
    options
        .params
        .insert("series_name".to_string(), LabelParamValue::Str("Revenue".to_string()));
    options
        .params
        .insert("threshold".to_string(), LabelParamValue::Float(-2.5));

    let label = engine().compile("#series_name >= #threshold", &options).unwrap();

    // Numbers display with upstream's minus sign.
    assert_eq!(label.semantic_text(), "Revenue >= −2.5");
    assert!(!label.flags.has_math);
}

#[test]
fn compile_resolves_text_params_inside_static_markup() {
    let mut options = LabelOptions::default();
    options
        .params
        .insert("series_name".to_string(), LabelParamValue::Str("revenue".to_string()));

    let label = engine().compile("#upper[#series_name]", &options).unwrap();

    assert_eq!(label.semantic_text(), "REVENUE");
}

#[test]
fn compile_lower_uses_context_sensitive_unicode_casing() {
    let label = engine().compile("#lower[ΟΣ]", &LabelOptions::default()).unwrap();

    assert_eq!(label.semantic_text(), "ος");
}

#[test]
fn compile_text_treats_param_syntax_as_literal_text() {
    let mut options = LabelOptions::default();
    options
        .params
        .insert("series_name".to_string(), LabelParamValue::Str("Revenue".to_string()));

    let label = engine().compile_text("#series_name", &options).unwrap();

    assert_eq!(label.semantic_text(), "#series_name");
}

#[test]
fn compile_errors_for_unknown_text_param() {
    let (message, range, _) = error("#series_name", &LabelOptions::default());
    assert_eq!((message.as_str(), range), ("unknown variable: series_name", 1..12));
}

#[test]
fn compile_numfmt_uses_number_locale_context() {
    let number: Arc<dyn NumberFormatProvider> = Arc::new(
        D3NumberFormatProvider::new().with_locale("de-DE").with_custom_locale(
            "de-DE",
            serde_json::from_str(include_str!(
                "../../avenger-format-number-d3/locales/de-DE.json"
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
            LabelFormatting { number: Some(&number), ..Default::default() },
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
fn params_that_dont_display_are_errors() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "items".to_string(),
        LabelParamValue::Array(vec![LabelParamValue::Int(1)]),
    );
    options
        .params
        .insert("active".to_string(), LabelParamValue::Bool(true));

    for source in ["#items", "$#items$"] {
        let (message, _, hints) = error(source, &options);
        assert_eq!(message, "cannot display array in a label", "{source}");
        assert_eq!(hints, ["use a string instead"], "{source}");
    }
    assert_eq!(error("#active", &options).0, "cannot display boolean in a label");
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

    let label = engine().compile("$y = #slope x + #intercept$", &options).unwrap();

    assert!(label.metrics.width > 0.0);
    assert!(label.metrics.height > 0.0);
    assert!(label.flags.has_math);
}

#[test]
fn math_names_stay_in_math_namespace_when_params_exist() {
    let engine = engine();
    let source = "$alpha + frac(1, 2) + sqrt(x) + bold(x)$";
    let baseline = engine.compile(source, &LabelOptions::default()).unwrap();

    let mut options = LabelOptions::default();
    options
        .params
        .insert("series_name".to_string(), LabelParamValue::Str("param".to_string()));

    let with_params = engine.compile(source, &options).unwrap();

    assert_metrics_close(with_params.metrics.width, baseline.metrics.width);
    assert_metrics_close(with_params.metrics.height, baseline.metrics.height);
    assert_metrics_close(with_params.metrics.baseline, baseline.metrics.baseline);
}

#[test]
fn params_shadow_library_names() {
    let mut options = LabelOptions::default();
    for name in ["upper", "frac"] {
        options
            .params
            .insert(name.to_string(), LabelParamValue::Str(name.to_uppercase()));
    }
    assert_eq!(engine().compile("#upper", &options).unwrap().semantic_text(), "UPPER");
    assert_eq!(engine().compile("$frac$", &options).unwrap().semantic_text(), "FRAC");
}

#[test]
fn compile_text_allows_param_names_colliding_with_markup_names() {
    let mut options = LabelOptions::default();
    options
        .params
        .insert("upper".to_string(), LabelParamValue::Str("param".to_string()));
    options
        .params
        .insert("frac".to_string(), LabelParamValue::Str("param".to_string()));

    let label = engine().compile_text("#upper and frac", &options).unwrap();

    assert_eq!(label.semantic_text(), "#upper and frac");
}

#[test]
fn string_params_name_colors_through_rgb() {
    let mut options = LabelOptions::default();
    options
        .params
        .insert("series_color".to_string(), LabelParamValue::Str("tomato".to_string()));

    let label = engine()
        .compile("#underline(stroke: 1.5pt + rgb(series_color))[Series]", &options)
        .unwrap();
    let stroke = first_stroke(&label);

    // CSS colors (D22).
    assert_eq!(stroke.paint.to_rgba8(), [255, 99, 71, 255]);
    assert_metrics_close(stroke.thickness, 1.5);
}

#[test]
fn named_colors_are_css_colors() {
    // CSS colors (D22). Upstream's palette differs: its `red` is `#ff4136` and its `green`
    // `#2ecc40`.
    for (source, rgba) in [
        ("#underline(stroke: red)[abc]", [255, 0, 0, 255]),
        ("#underline(stroke: 1.2pt + green)[abc]", [0, 128, 0, 255]),
        ("#strike(stroke: tomato)[abc]", [255, 99, 71, 255]),
        ("$cancel(x, stroke: #(paint: maroon, cap: \"round\"))$", [128, 0, 0, 255]),
        ("#overline(stroke: rgb(\"rebeccapurple\"))[abc]", [102, 51, 153, 255]),
        ("#underline(stroke: rgb(\"hsl(120, 100%, 25%)\"))[abc]", [0, 128, 0, 255]),
    ] {
        let label = engine().compile(source, &LabelOptions::default()).unwrap();
        assert_eq!(first_stroke(&label).paint.to_rgba8(), rgba, "{source}");
    }
    let label = engine()
        .compile("#text(fill: navy)[abc]", &LabelOptions::default())
        .unwrap();
    let text = label.frame.text_items()[0].1;
    assert_eq!(text.fill.to_rgba8(), [0, 0, 128, 255]);
}

#[test]
fn dictionary_params_are_strokes() {
    let mut stroke_param = IndexMap::new();
    stroke_param.insert("cap".to_string(), LabelParamValue::Str("round".to_string()));
    stroke_param.insert("join".to_string(), LabelParamValue::Str("bevel".to_string()));
    stroke_param.insert("dash".to_string(), LabelParamValue::Str("dashed".to_string()));
    stroke_param.insert("miter-limit".to_string(), LabelParamValue::Float(2.0));

    let mut options = LabelOptions::default();
    options.text.fill = AbsoluteColor::from_srgb(0.0, 0.0, 1.0, 1.0);
    options
        .params
        .insert("series_stroke".to_string(), LabelParamValue::Dict(stroke_param));

    let label = engine()
        .compile("#underline(stroke: series_stroke)[Series]", &options)
        .unwrap();
    let stroke = first_stroke(&label);

    // The stroke takes the text's fill and the font's underline thickness.
    assert_eq!(stroke.paint.to_rgba8(), [0, 0, 255, 255]);
    assert!(stroke.thickness > 0.0);
    assert_eq!(stroke.cap, LineCap::Round);
    assert_eq!(stroke.join, LineJoin::Bevel);
    let dash = stroke.dash.as_ref().expect("dash should resolve");
    assert_eq!(dash.array.len(), 2);
    assert_eq!(dash.phase, 0.0);
    assert_eq!(stroke.miter_limit, 2.0);
}

#[test]
fn compile_mixed_label_returns_text_and_shapes() {
    let label = engine()
        .compile("Price \\$7, ratio $a / b$ = 0.94", &LabelOptions::default())
        .unwrap();

    assert!(label.metrics.width > 0.0);
    assert!(label.metrics.height > 0.0);
    assert!(label.flags.has_math);
    let items = items(&label);
    assert!(items.iter().any(|item| matches!(item, FrameItem::Text(_))));
    assert!(items.iter().any(|item| matches!(item, FrameItem::Shape(_))));
}

#[test]
fn svg_and_pdf_lowerers_consume_compiled_label() {
    let label = engine()
        .compile("Price \\$7, ratio $frac(a, b)$ = 0.94", &LabelOptions::default())
        .unwrap();

    let svg = svg_items(&label, &SvgOptions::default());
    assert_eq!(svg.size, label.frame.size);
    assert!(svg.items.iter().any(|item| matches!(item, SvgItem::Path(_))));

    let pdf = pdf_items(&label, &PdfOptions::default());
    assert!(pdf.items.iter().any(|item| matches!(item, PdfItem::Text(_))));
    assert!(pdf.items.iter().any(|item| matches!(item, PdfItem::Path(_))));
    assert!(pdf.fonts.len() >= 2);
    assert_eq!(pdf.semantic_text, label.semantic_text());
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
    let engine = engine().with_datetime_formatting(Arc::new(
        D3DateTimeFormatProvider::new().with_timezone(Tokyo),
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
            Err(LabelError::Source { message, .. }) if message.contains(expected)
        ));
    }
}

#[test]
fn datetime_cache_tracks_request_configuration_and_input_type() {
    let engine = engine();
    let provider = D3DateTimeFormatProvider::new().with_custom_locale(
        "fr-FR",
        serde_json::from_str(include_str!(
            "../../avenger-format-datetime-d3/locales/fr-FR.json"
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
        ("%B %d %H:%M", "en-US", UTC, "January 01 00:00"),
        ("%B %d %H:%M", "fr-FR", UTC, "janvier 01 00:00"),
        ("%B %d %H:%M", "fr-FR", New_York, "décembre 31 19:00"),
        ("%Y %Z", "fr-FR", New_York, "2023 -0500"),
    ] {
        options
            .params
            .insert("pattern".into(), LabelParamValue::Str(pattern.into()));
        let datetime: Arc<dyn DateTimeFormatProvider> =
            Arc::new(provider.clone().with_locale(locale).with_timezone(timezone));
        assert_eq!(
            engine
                .compile_with_formatting(
                    "#datefmt(value, pattern)",
                    &options,
                    LabelFormatting { datetime: Some(&datetime), ..Default::default() }
                )
                .unwrap()
                .semantic_text(),
            expected
        );
    }
    options
        .params
        .insert("pattern".into(), LabelParamValue::Str("%Y".into()));
    let datetime: Arc<dyn DateTimeFormatProvider> =
        Arc::new(provider.with_timezone(New_York));
    let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    for (value, expected) in [
        (LabelParamValue::Date(date), "2024"),
        (
            LabelParamValue::UtcDateTime(date.and_hms_opt(0, 0, 0).unwrap().and_utc()),
            "2023",
        ),
        (LabelParamValue::DateTime(date.and_hms_opt(0, 0, 0).unwrap()), "2024"),
    ] {
        options.params.insert("value".into(), value);
        assert_eq!(
            engine
                .compile_with_formatting(
                    "#datefmt(value, pattern)",
                    &options,
                    LabelFormatting { datetime: Some(&datetime), ..Default::default() }
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Debug)]
    struct Provider {
        calls: Arc<AtomicUsize>,
        unit: String,
    }
    #[derive(Debug)]
    struct Prepared(String);
    impl NumberFormatProvider for Provider {
        fn prepare(
            &self,
            pattern: &str,
        ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(Prepared(format!("{pattern} {}", self.unit))))
        }
    }
    impl PreparedNumberFormatter for Prepared {
        fn format(&self, value: f64) -> FormattedNumber {
            FormattedNumber::plain(format!("{}: {value}", self.0))
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn NumberFormatProvider> =
        Arc::new(Provider { calls: calls.clone(), unit: "items".into() });
    let first = engine().with_number_formatting(provider.clone());
    let mut options = LabelOptions::default();
    options
        .params
        .insert("pattern".into(), LabelParamValue::Str("custom syntax".into()));
    let source = "#numfmt(2, pattern)";
    for engine in [&first, &first.clone().with_number_formatting(provider)] {
        assert_eq!(
            engine.compile(source, &options).unwrap().semantic_text(),
            "custom syntax items: 2"
        );
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let other: Arc<dyn NumberFormatProvider> =
        Arc::new(Provider { calls: calls.clone(), unit: "widgets".into() });
    assert_eq!(
        first
            .compile_with_formatting(
                source,
                &options,
                LabelFormatting { number: Some(&other), ..Default::default() }
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
    let engine = LabelEngine::new(common::engine_options());
    let options = LabelOptions::default();
    assert_eq!(engine.compile("Revenue", &options).unwrap().semantic_text(), "Revenue");
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
    let engine = LabelEngine::new(common::engine_options());
    let options = LabelOptions {
        params: [(
            "value".into(),
            LabelParamValue::UtcDateTime(chrono::DateTime::UNIX_EPOCH),
        )]
        .into(),
        ..Default::default()
    };
    assert_eq!(engine.compile("Date", &options).unwrap().semantic_text(), "Date");
    assert!(
        engine
            .compile("#datefmt(value, \"%Y\")", &options)
            .unwrap_err()
            .to_string()
            .contains("datetime formatting is not configured")
    );
}
