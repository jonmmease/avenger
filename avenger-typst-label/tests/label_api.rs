mod common;

use avenger_typst_label::{
    CompiledLabel, CurveItem, FrameItem, GroupItem, LabelEngine, LabelError, LabelFrame,
    LabelOptions, LabelParamValue, LineCap, LineJoin, PathKind, PdfItem, PdfOptions,
    Stroke, SvgItem, SvgOptions, escape_text, pdf_items, svg_items,
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

/// A label's frame without its source ranges, which differ between equivalent sources.
fn layout(label: &CompiledLabel) -> LabelFrame {
    fn strip(frame: &LabelFrame) -> LabelFrame {
        let items = frame.items.iter().map(|(pos, item)| {
            let item = match item {
                FrameItem::Group(group) => FrameItem::Group(GroupItem {
                    frame: strip(&group.frame),
                    transform: group.transform,
                }),
                FrameItem::Text(text) => {
                    let mut text = text.clone();
                    text.source = 0..0;
                    for glyph in &mut text.glyphs {
                        glyph.source = 0..0;
                    }
                    FrameItem::Text(text)
                }
                FrameItem::Shape(shape) => FrameItem::Shape(shape.clone()),
            };
            (*pos, item)
        });
        LabelFrame {
            size: frame.size,
            baseline: frame.baseline,
            items: items.collect(),
        }
    }
    strip(&label.frame)
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

/// Literal text lays out as its escaped markup does, and measures as it compiles.
#[test]
fn compile_text_renders_as_escaped_markup() {
    let engine = engine();
    let options = LabelOptions::default();
    for text in [
        "Revenue by region",
        "cost $5 #not-markup [brackets]",
        "*literal* _literal_ `literal`",
        "a~b a-b a...b",
        "user@host <tag> /path [brackets]",
        "- item + item = value :colon",
        "\"quote\" 'quote' http://example.com #hash $dollar",
        "Revenue 🚀 שלום नमस्ते",
        // Runs of whitespace are one space, and line breaks are spaces (D25).
        "a   b",
        "a\tb",
        "a\nb",
        "a\r\n\r\nb",
        "  a b  ",
    ] {
        let literal = engine.compile_text(text, &options).unwrap();
        let escaped = engine.compile(&escape_text(text), &options).unwrap();
        assert_metrics_close(literal.metrics.width, escaped.metrics.width);
        assert_metrics_close(literal.metrics.height, escaped.metrics.height);
        assert_metrics_close(literal.metrics.baseline, escaped.metrics.baseline);
        assert_eq!(literal.semantic_text, escaped.semantic_text, "{text:?}");
        assert!(!literal.flags.has_math, "{text:?}");
        assert_eq!(engine.measure_text(text, &options).unwrap(), literal.metrics);
    }
    // Straight quotes and markup characters stay as they are.
    let text = "\"quote\" 'quote' cost $5 #hash [brackets]";
    assert_eq!(engine.compile_text(text, &options).unwrap().semantic_text, text);
    assert_eq!(
        engine.compile_text("a \n\n b", &options).unwrap().metrics,
        engine.compile_text("a b", &options).unwrap().metrics
    );
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
        assert!(label.semantic_text.contains('🚀'), "{source}");
        assert!(label.semantic_text.contains("שלום"), "{source}");
        assert!(label.semantic_text.contains("नमस्ते"), "{source}");
    }
}

#[test]
fn compile_resolves_named_emoji_and_symbol_aliases() {
    let label = engine()
        .compile("Trend #emoji.chart.up #sym.arrow.r target", &LabelOptions::default())
        .unwrap();

    assert_eq!(label.semantic_text, "Trend 📈 → target");
    assert!(label.metrics.width > 0.0);

    // Modifiers in any order, and symbols in modules nested in `sym`.
    let label = engine()
        .compile(
            "#sym.gt.eq.not #sym.gt.not.eq #sym.forces.not #sym.gender.male.stroke.t \
             #sym.control.dc.three",
            &LabelOptions::default(),
        )
        .unwrap();
    assert_eq!(label.semantic_text, "≱ ≱ ⊮ ⚨ ␓");
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
    assert_eq!(label.semantic_text, "Revenue >= −2.5");
    assert!(!label.flags.has_math);
}

#[test]
fn compile_resolves_text_params_inside_static_markup() {
    let mut options = LabelOptions::default();
    options
        .params
        .insert("series_name".to_string(), LabelParamValue::Str("revenue".to_string()));

    let label = engine().compile("#upper[#series_name]", &options).unwrap();

    assert_eq!(label.semantic_text, "REVENUE");
}

#[test]
fn compile_lower_uses_context_sensitive_unicode_casing() {
    let label = engine().compile("#lower[ΟΣ]", &LabelOptions::default()).unwrap();

    assert_eq!(label.semantic_text, "ος");
}

#[test]
fn compile_text_treats_param_syntax_as_literal_text() {
    let mut options = LabelOptions::default();
    options
        .params
        .insert("series_name".to_string(), LabelParamValue::Str("Revenue".to_string()));

    let label = engine().compile_text("#series_name", &options).unwrap();

    assert_eq!(label.semantic_text, "#series_name");
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

    assert_eq!(label.semantic_text, "1.234,5");

    // A localized mantissa stays one number in math, where a comma would be punctuation.
    let label = engine()
        .compile_with_formatting(
            "#underline[#numfmt(value, \".1e\")]",
            &options,
            LabelFormatting { number: Some(&number), ..Default::default() },
        )
        .unwrap();
    assert!(label.flags.has_math);
    let texts: Vec<_> = label
        .frame
        .text_items()
        .into_iter()
        .map(|(_, text)| text.text.as_str())
        .collect();
    assert!(texts.contains(&"1,2"), "{texts:?}");
}

#[test]
fn compile_datetimefmt_formats_temporal_param() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "report_date".to_string(),
        LabelParamValue::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()),
    );

    let label = engine()
        .compile("Report #datetimefmt(report_date, \"%B %-d, %Y\")", &options)
        .unwrap();

    assert_eq!(label.semantic_text, "Report January 5, 2024");
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
    assert_eq!(label.semantic_text, "𝑦=2.5𝑥+7");
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
    assert_eq!(engine().compile("#upper", &options).unwrap().semantic_text, "UPPER");
    assert_eq!(engine().compile("$frac$", &options).unwrap().semantic_text, "FRAC");
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

    assert_eq!(label.semantic_text, "#upper and frac");
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
fn option_params_cast_like_upstream_values() {
    let mut options = LabelOptions::default();
    let mut param = |name: &str, value| options.params.insert(name.into(), value);
    // Strings and booleans don't cast to lengths: casts are upstream's.
    param("offset_text", LabelParamValue::Str("2pt".into()));
    param("flag", LabelParamValue::Bool(true));
    // A number times a unit is a length, and booleans are booleans.
    param("offset", LabelParamValue::Float(2.0));
    param("extent", LabelParamValue::Float(-0.5));
    param("background", LabelParamValue::Bool(true));
    param("evade", LabelParamValue::Bool(false));
    param("typographic", LabelParamValue::Bool(false));
    param("baseline", LabelParamValue::Float(-0.25));
    param("size", LabelParamValue::Float(8.0));
    param("all", LabelParamValue::Bool(true));

    for (source, message, range) in [
        (
            "#underline(offset: offset_text)[care]",
            "expected length or auto, found string",
            19..30,
        ),
        (
            "#underline(offset: flag)[care]",
            "expected length or auto, found boolean",
            19..23,
        ),
        ("#super(size: offset_text)[N]", "expected length or auto, found string", 13..24),
    ] {
        let (actual, actual_range, hints) = error(source, &options);
        assert_eq!((actual.as_str(), actual_range), (message, range), "{source}");
        assert!(hints.is_empty(), "{source}");
    }

    for (with_params, literal) in [
        (
            "#underline(offset: offset * 1pt, extent: extent * 1em, background: background, \
             evade: evade)[care]",
            "#underline(offset: 2pt, extent: -0.5em, background: true, evade: false)[care]",
        ),
        (
            "#super(typographic: typographic, baseline: baseline * 1em, size: size * 1pt)[N] \
             #smallcaps(all: all)[UNICEF]",
            "#super(typographic: false, baseline: -0.25em, size: 8pt)[N] \
             #smallcaps(all: true)[UNICEF]",
        ),
    ] {
        // One engine, since font references compare font instances.
        let engine = engine();
        let actual = engine.compile(with_params, &options).unwrap();
        let expected = engine.compile(literal, &options).unwrap();
        assert_eq!(layout(&actual), layout(&expected), "{with_params}");
    }
}

#[test]
fn formatting_functions_take_a_value_and_a_pattern() {
    let mut options = LabelOptions::default();
    options.params.insert(
        "value".into(),
        LabelParamValue::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()),
    );
    // Locales and timezones are the providers' settings.
    for name in ["locale", "timezone", "tz"] {
        let source = format!("#datetimefmt(value, \"%Y\", {name}: \"UTC\")");
        let (message, range, _) = error(&source, &options);
        assert_eq!(
            (message, range),
            (format!("unexpected argument: {name}"), 26..33 + name.len())
        );
    }
    let (message, range, _) = error("#numfmt(1, \"f\", precision: 2)", &options);
    assert_eq!((message.as_str(), range), ("unexpected argument: precision", 16..28));
}

#[test]
fn param_glyphs_map_to_their_identifier() {
    let mut options = LabelOptions::default();
    options
        .params
        .insert("series_name".into(), LabelParamValue::Str("Revenue".into()));
    let label = engine().compile("Series #series_name", &options).unwrap();
    let sources: Vec<_> = label
        .frame
        .text_items()
        .into_iter()
        .flat_map(|(_, text)| text.glyphs.iter().map(|glyph| glyph.source.clone()))
        .collect();
    // Verbatim text maps byte for byte; a parameter's text maps to its name, without the `#`.
    assert_eq!(sources[..7], [0..1, 1..2, 2..3, 3..4, 4..5, 5..6, 6..7]);
    assert_eq!(sources[7..], vec![8..19; 7], "{sources:?}");
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
fn lowerers_draw_each_text_item_and_list_each_font_once() {
    let engine = engine();
    for (source, fonts) in [
        ("Hello", 1),
        ("$alpha + beta -> gamma$", 1),
        ("$R^2 = 0.94$", 1),
        ("Price \\$7, score $R^2$ = 0.94", 2),
    ] {
        let label = engine.compile(source, &LabelOptions::default()).unwrap();
        let texts = label.frame.text_items();

        // A PDF run per text item, in its font.
        let pdf = pdf_items(&label, &PdfOptions::default());
        assert_eq!(pdf.fonts.len(), fonts, "{source}");
        let runs: Vec<_> = pdf
            .items
            .iter()
            .filter_map(|item| match item {
                PdfItem::Text(run) => Some(run),
                _ => None,
            })
            .collect();
        assert_eq!(runs.len(), texts.len(), "{source}");
        for (run, (_, text)) in runs.iter().zip(&texts) {
            assert_eq!(pdf.fonts[run.font], text.font, "{source}");
            assert_eq!((&run.text, run.glyphs.len()), (&text.text, text.glyphs.len()));
        }

        // An SVG path per glyph with an outline: every glyph but spaces.
        let svg = svg_items(&label, &SvgOptions::default());
        let paths = svg
            .items
            .iter()
            .filter(|item| matches!(item, SvgItem::Path(path) if matches!(path.kind, PathKind::Glyph(_))))
            .count();
        let inked = texts
            .iter()
            .flat_map(|(_, text)| {
                text.glyphs.iter().map(|glyph| &text.text[glyph.range.clone()])
            })
            .filter(|cluster| !cluster.trim().is_empty())
            .count();
        assert_eq!(paths, inked, "{source}");
    }
}

#[test]
fn rectangles_keep_upstreams_winding() {
    let label = engine().compile("#highlight[abc]", &LabelOptions::default()).unwrap();
    let svg = svg_items(&label, &SvgOptions::default());
    let rect = svg
        .items
        .iter()
        .find_map(|item| match item {
            SvgItem::Path(path) if path.kind == PathKind::Shape => Some(&path.path),
            _ => None,
        })
        .unwrap();
    let [
        CurveItem::Move(start),
        CurveItem::Line(a),
        CurveItem::Line(b),
        CurveItem::Line(c),
        CurveItem::Close,
    ] = rect.0[..]
    else {
        panic!("{rect:?}");
    };
    // Down the left side first, as upstream's renderers draw a rectangle.
    assert_eq!((start.x, start.y, a.x), (0.0, 0.0, 0.0));
    assert!(a.y > 0.0 && b == avenger_typst_label::Point::new(c.x, a.y) && c.y == 0.0);
}

#[test]
fn semantic_text_reads_text_logically_and_math_as_drawn() {
    let engine = engine();
    for (source, expected) in [
        // Accents and primes follow their base.
        ("$hat(x)$", "𝑥\u{302}"),
        ("$x'$", "𝑥′"),
        ("a $tilde(a b)$ b", "a 𝑎𝑏\u{303} b"),
        // Limits and stretched glyphs take their place in the equation.
        ("$a + lim_(x -> 0) f(x)$", "𝑎+lim𝑥→0𝑓(𝑥)"),
        ("$P -> Q stretch(->, size: #200%) R$", "𝑃→𝑄→𝑅"),
        ("$sum_(i=1)^n i$", "∑𝑛𝑖=1𝑖"),
        // Right-to-left text reads in logical order, without the embeddings around it.
        ("abc #underline[אבג 123] xyz", "abc אבג 123 xyz"),
        ("#text(lang: \"he\")[abc אבג]", "abc אבג"),
    ] {
        let label = engine.compile(source, &LabelOptions::default()).unwrap();
        assert_eq!(label.semantic_text, expected, "{source}");
    }
}

/// Explicit breaks end lines. The label takes its first line's baseline, its text has a newline
/// at each break, and it measures as it compiles.
#[test]
fn explicit_breaks_end_lines() {
    let engine = engine();
    let options = LabelOptions::default();
    let line = engine.compile("Revenue", &options).unwrap().metrics;
    for (source, text) in [
        ("Revenue \\ (millions)", "Revenue\n(millions)"),
        ("Revenue #linebreak() (millions)", "Revenue\n(millions)"),
        ("a \\ \\ b", "a\n\nb"),
    ] {
        let label = engine.compile(source, &options).unwrap();
        assert_eq!(label.semantic_text, text, "{source}");
        let metrics = label.metrics;
        assert_eq!(
            (metrics.baseline, metrics.ascent),
            (line.baseline, line.ascent),
            "{source}"
        );
        assert!(metrics.height > 2.0 * line.height, "{source}");
        assert_eq!(engine.measure(source, &options).unwrap(), metrics, "{source}");
    }
    // A break at the end ends the only line.
    let label = engine.compile("Revenue \\", &options).unwrap();
    assert_eq!((label.metrics, label.semantic_text.as_str()), (line, "Revenue"));

    // Line breaks in data are spaces.
    let mut options = LabelOptions::default();
    options
        .params
        .insert("name".into(), LabelParamValue::Str("a\nb".into()));
    let spaced = engine.compile("a b", &options).unwrap().metrics;
    for source in ["#name", "#\"a\\nb\"", "a\\u{a}b"] {
        let label = engine.compile(source, &options).unwrap();
        assert_eq!(
            (label.metrics, label.semantic_text.as_str()),
            (spaced, "a b"),
            "{source}"
        );
    }
}

#[test]
fn empty_labels_are_empty() {
    let engine = engine();
    let options = LabelOptions::default();
    for label in [engine.compile("", &options), engine.compile_text("", &options)] {
        let label = label.unwrap();
        assert_eq!((label.metrics.width, label.metrics.height), (0.0, 0.0));
        assert!(label.frame.items.is_empty());
        assert_eq!(label.semantic_text, "");
        assert!(svg_items(&label, &SvgOptions::default()).items.is_empty());
        let pdf = pdf_items(&label, &PdfOptions::default());
        assert!(pdf.items.is_empty() && pdf.fonts.is_empty());
        #[cfg(feature = "raster")]
        {
            // A single transparent pixel.
            let raster = rasterize(&label, &RasterOptions::default()).unwrap();
            assert_eq!((raster.image.width, raster.image.height), (1, 1));
            assert_eq!(raster.image.data, [0; 4]);
        }
    }
    assert_eq!(engine.measure("", &options).unwrap().width, 0.0);
}

#[test]
fn datetimefmt_reports_value_errors_through_label_compilation() {
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
            LabelParamValue::ZonedDateTime(chrono::DateTime::<chrono::Utc>::MAX_UTC),
            "calendar range",
        ),
        (LabelParamValue::NaiveDateTime(leap), "leap seconds"),
    ] {
        options.params.insert("value".into(), value);
        assert!(matches!(
            engine.compile("#datetimefmt(value, \"%Y\")", &options),
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
        LabelParamValue::ZonedDateTime(
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
                    "#datetimefmt(value, pattern)",
                    &options,
                    LabelFormatting { datetime: Some(&datetime), ..Default::default() }
                )
                .unwrap()
                .semantic_text,
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
            LabelParamValue::ZonedDateTime(date.and_hms_opt(0, 0, 0).unwrap().and_utc()),
            "2023",
        ),
        (LabelParamValue::NaiveDateTime(date.and_hms_opt(0, 0, 0).unwrap()), "2024"),
    ] {
        options.params.insert("value".into(), value);
        assert_eq!(
            engine
                .compile_with_formatting(
                    "#datetimefmt(value, pattern)",
                    &options,
                    LabelFormatting { datetime: Some(&datetime), ..Default::default() }
                )
                .unwrap()
                .semantic_text,
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
            engine.compile(source, &options).unwrap().semantic_text,
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
            .semantic_text,
        "custom syntax widgets: 2"
    );
    options
        .params
        .insert("pattern".into(), LabelParamValue::Str("updated".into()));
    assert_eq!(
        first.compile(source, &options).unwrap().semantic_text,
        "updated items: 2"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[test]
fn numeric_markup_requires_explicit_formatting_but_plain_text_does_not() {
    let engine = LabelEngine::new(common::engine_options());
    let options = LabelOptions::default();
    assert_eq!(engine.compile("Revenue", &options).unwrap().semantic_text, "Revenue");
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
            LabelParamValue::ZonedDateTime(chrono::DateTime::UNIX_EPOCH),
        )]
        .into(),
        ..Default::default()
    };
    assert_eq!(engine.compile("Date", &options).unwrap().semantic_text, "Date");
    assert!(
        engine
            .compile("#datetimefmt(value, \"%Y\")", &options)
            .unwrap_err()
            .to_string()
            .contains("datetime formatting is not configured")
    );
}

#[test]
#[cfg(target_os = "macos")]
fn emoji_are_bitmap_glyphs_in_one_cluster() {
    // With the system's fonts, the emoji fall back to Apple Color Emoji.
    let mut options = common::engine_options();
    options.fonts.load_system_fonts = true;
    let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
    let label = LabelEngine::new(options)
        .compile(&format!("Family {family}"), &LabelOptions::default())
        .unwrap();
    let (_, emoji) = label
        .frame
        .text_items()
        .into_iter()
        .find(|(_, text)| text.font.family() == "Apple Color Emoji")
        .expect("Apple Color Emoji covers the emoji");
    // A zero-width-joiner sequence is one glyph and one cluster.
    assert_eq!(emoji.glyphs.len(), 1);
    assert_eq!(emoji.glyphs[0].range, 0..family.len());
    assert_eq!(emoji.glyphs[0].source, 7..7 + family.len());

    // Its glyph is a bitmap, so it lowers to an image, except in PDF runs, which keep it.
    let svg = svg_items(&label, &SvgOptions::default());
    assert!(svg.items.iter().any(|item| matches!(item, SvgItem::Image(_))));
    let pdf = pdf_items(&label, &PdfOptions::default());
    assert!(pdf.items.iter().any(|item| matches!(
        item,
        PdfItem::Text(run) if run.text == family && run.glyphs.len() == 1
    )));
    #[cfg(feature = "raster")]
    {
        let raster = rasterize(&label, &RasterOptions::default()).unwrap();
        let colored = raster
            .image
            .data
            .chunks_exact(4)
            .filter(|pixel| {
                pixel[3] > 0 && (pixel[0] != pixel[1] || pixel[1] != pixel[2])
            })
            .count();
        assert!(colored > 20, "{colored} colored pixels");
    }
}
