mod common;

use avenger_typst_label::{
    CompiledLabel, CurveItem, FrameItem, GroupItem, LabelAlign, LabelEngine, LabelError,
    LabelFrame, LabelLineHeight, LabelMetrics, LabelOptions, LabelParamValue, LabelWidth,
    LineCap, LineJoin, LineMetrics, PathKind, PdfItem, PdfOptions, Stroke, SvgItem,
    SvgOptions, TextDir, TextRun, escape_text, pdf_items, svg_items,
};
use indexmap::IndexMap;
use std::num::NonZeroUsize;
use unicode_segmentation::UnicodeSegmentation;

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

/// A label's glyphs as their ids, positions in the label and advances in points, top to bottom
/// and left to right.
fn placed_glyphs(label: &CompiledLabel) -> Vec<(u16, f32, f32, f32)> {
    let mut glyphs = vec![];
    label.frame.visit(Default::default(), &mut |ts, item| {
        if let FrameItem::Text(text) = item {
            for (pos, glyph) in text.positioned_glyphs() {
                let advance = glyph.x_advance * text.size;
                glyphs.push((glyph.id, ts.tx + pos.x, ts.ty + pos.y, advance));
            }
        }
    });
    glyphs.sort_by(|a, b| (a.2, a.1).partial_cmp(&(b.2, b.1)).unwrap());
    glyphs
}

/// Whether glyphs are the same, within 0.01 points.
fn same_glyphs(a: &[(u16, f32, f32, f32)], b: &[(u16, f32, f32, f32)]) -> bool {
    let close = |a: f32, b: f32| (a - b).abs() < 0.01;
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.0 == b.0 && close(a.1, b.1) && close(a.2, b.2) && close(a.3, b.3)
        })
}

/// Where a line lies down its label: its top, baseline and bottom.
fn vertical(line: &LineMetrics) -> (f32, f32, f32) {
    (line.top, line.baseline, line.bottom)
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
        let first = |label: &CompiledLabel| label.metrics.lines[0].baseline;
        assert_metrics_close(first(&literal), first(&escaped));
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
    assert_metrics_close(
        with_params.metrics.lines[0].baseline,
        baseline.metrics.lines[0].baseline,
    );
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
        ("Revenue \\ (millions, $R^2$)", 2),
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
        for (run, (ts, text)) in runs.iter().zip(&texts) {
            assert_eq!(pdf.fonts[run.font], text.font, "{source}");
            assert_eq!((&run.text, run.glyphs.len()), (&text.text, text.glyphs.len()));
            // On its line's baseline.
            assert!((run.transform.ty - ts.ty).abs() < 1e-3, "{source}");
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

/// With native text, text items that viewers draw as the label does lower to runs, with their
/// faces' weights and styles, on their baselines and in drawing order. Substituted glyphs and
/// math stay outlines, while synthesized scripts, faces without the features they ask for and
/// wrapped lines stay text.
#[test]
fn native_text_lowers_to_runs() {
    let engine = engine();
    let mut lato = LabelOptions::default();
    lato.text.font_family = "Lato".into();
    lato.text.font_size = 40.0;
    // The runs and the number of glyph outlines.
    let lower = |source: &str, options: &LabelOptions| {
        let label = engine.compile(source, options).unwrap();
        let items = svg_items(&label, &SvgOptions { native_text: true }).items;
        let runs: Vec<TextRun> = items
            .iter()
            .filter_map(|item| match item {
                SvgItem::Text(run) => Some(run.clone()),
                _ => None,
            })
            .collect();
        let outlines = items
            .iter()
            .filter(|item| matches!(item, SvgItem::Path(path) if matches!(path.kind, PathKind::Glyph(_))))
            .count();
        (label, runs, outlines)
    };
    let texts = |runs: &[TextRun]| -> Vec<String> {
        runs.iter().map(|run| run.text.clone()).collect()
    };

    // Each run has its face's weight and style, and sits on its item's baseline.
    let (label, runs, outlines) = lower("Regular _Italic_ *Bold*", &lato);
    assert_eq!((texts(&runs).concat().as_str(), outlines), ("Regular Italic Bold", 0));
    let items = label.frame.text_items();
    for run in &runs {
        let (ts, item) = &items[run.text_item];
        assert_eq!((&run.text, run.baseline, run.x), (&item.text, ts.ty, ts.tx));
        assert_eq!(run.font.family(), "Lato");
        let italic = run.text == "Italic";
        assert_eq!(
            run.style == avenger_typst_label::FontStyle::Italic,
            italic,
            "{run:?}"
        );
        if run.text == "Bold" {
            assert_eq!(run.weight.to_number(), 700);
        }
    }
    // Without native text, every glyph is an outline.
    let svg = svg_items(&label, &SvgOptions::default());
    assert!(!svg.items.iter().any(|item| matches!(item, SvgItem::Text(_))));

    // Lato's typographic scripts substitute script glyphs, which stay outlines; synthesized
    // scripts are smaller text of the same glyphs, below or above the baseline. Lato has no
    // small capitals, so small caps keep the ordinary glyphs.
    let (_, runs, outlines) = lower("H#sub[2]O", &lato);
    assert_eq!((texts(&runs).concat().as_str(), outlines), ("HO", 1));
    let (_, runs, outlines) = lower("H#sub(typographic: false)[2]O", &lato);
    assert_eq!((texts(&runs), outlines), (vec!["H".into(), "2".into(), "O".into()], 0));
    assert!(runs[1].size < runs[0].size && runs[1].baseline > runs[0].baseline);
    let (_, runs, outlines) = lower("#smallcaps[Smallcaps]", &lato);
    assert_eq!((texts(&runs), outlines), (vec!["Smallcaps".into()], 0));

    // Math faces stay outlines.
    let (_, runs, outlines) = lower("speed $v^2$", &LabelOptions::default());
    assert_eq!(texts(&runs), ["speed "]);
    assert!(outlines >= 2);

    // Runs come in drawing order, with a run of spaces in its place.
    let (_, runs, _) =
        lower("#underline[Decorations] _Italic_", &LabelOptions::default());
    assert_eq!(texts(&runs), ["Decorations", " ", "Italic"]);

    // A wrapped line's trailing space has no advance, and the line stays text.
    let source = "Revenue by region in millions of dollars";
    let options = LabelOptions {
        width: LabelWidth::Max(90.0),
        ..LabelOptions::default()
    };
    let (label, runs, outlines) = lower(source, &options);
    assert!(label.metrics.lines.len() > 2);
    assert_eq!((texts(&runs).concat().as_str(), outlines), (source, 0));
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

/// Explicit breaks end lines. The first line lies as a label's only line would, its text has a
/// newline at each break, and the label measures as it compiles.
#[test]
fn explicit_breaks_end_lines() {
    let engine = engine();
    let options = LabelOptions::default();
    let line = engine.compile("Revenue", &options).unwrap().metrics;
    for (source, text, lines) in [
        ("Revenue \\ (millions)", "Revenue\n(millions)", 2),
        ("Revenue #linebreak() (millions)", "Revenue\n(millions)", 2),
        ("a \\ \\ b", "a\n\nb", 3),
    ] {
        let label = engine.compile(source, &options).unwrap();
        assert_eq!(label.semantic_text, text, "{source}");
        let metrics = label.metrics;
        assert_eq!(metrics.lines.len(), lines, "{source}");
        assert_eq!(vertical(&metrics.lines[0]), vertical(&line.lines[0]), "{source}");
        assert_eq!(metrics.lines[lines - 1].bottom, metrics.height, "{source}");
        assert_eq!(engine.measure(source, &options).unwrap(), metrics, "{source}");
    }
    // A break at the end ends the only line.
    let label = engine.compile("Revenue \\", &options).unwrap();
    assert_eq!((&label.metrics, label.semantic_text.as_str()), (&line, "Revenue"));

    // Line breaks in data are spaces.
    let mut options = LabelOptions::default();
    options
        .params
        .insert("name".into(), LabelParamValue::Str("a\nb".into()));
    let spaced = engine.compile("a b", &options).unwrap().metrics;
    for source in ["#name", "#\"a\\nb\"", "a\\u{a}b"] {
        let label = engine.compile(source, &options).unwrap();
        assert_eq!(
            (&label.metrics, label.semantic_text.as_str()),
            (&spaced, "a b"),
            "{source}"
        );
    }
}

/// A width wraps lines greedily: a maximum width bounds the label, and a fixed one sets its
/// width. Wrapped lines keep their spaces in the text, and the label measures as it compiles.
#[test]
fn widths_wrap_lines() {
    let engine = engine();
    let source = "Revenue by region in millions of dollars";
    let mut options = LabelOptions::default();
    let line = engine.compile(source, &options).unwrap().metrics;
    for width in [LabelWidth::Max(90.0), LabelWidth::Fixed(90.0)] {
        options.width = width;
        let label = engine.compile(source, &options).unwrap();
        let metrics = label.metrics;
        assert_eq!(label.semantic_text, source, "{width:?}");
        assert_eq!(vertical(&metrics.lines[0]), vertical(&line.lines[0]), "{width:?}");
        assert!(metrics.lines.len() > 2, "{width:?}");
        assert_eq!(engine.measure(source, &options).unwrap(), metrics, "{width:?}");
        if let LabelWidth::Max(max) = width {
            assert!(metrics.width <= max, "{width:?}");
        } else {
            assert_eq!(metrics.width, 90.0);
        }
    }
    // Text within the width is one line, which a fixed width widens.
    options.width = LabelWidth::Max(1000.0);
    assert_eq!(engine.compile(source, &options).unwrap().metrics, line);
    options.width = LabelWidth::Fixed(1000.0);
    let metrics = engine.compile(source, &options).unwrap().metrics;
    assert_eq!((metrics.width, metrics.height), (1000.0, line.height));
    // At a width of zero, each word is a line.
    options.width = LabelWidth::Max(0.0);
    let metrics = engine.compile("a b c", &options).unwrap().metrics;
    let three = engine
        .compile("a \\ b \\ c", &LabelOptions::default())
        .unwrap()
        .metrics;
    assert_eq!((metrics.width, metrics.height), (0.0, three.height));
}

/// Without wrapping, lines end only at explicit breaks, as without a width, but the width still
/// bounds the label: a line wider than it overflows unsqueezed, and the label is no wider than
/// the width.
#[test]
fn unwrapped_lines_end_at_explicit_breaks() {
    let engine = engine();
    let source = "Revenue by region \\ in millions";
    let auto = engine.compile(source, &LabelOptions::default()).unwrap();
    for width in [LabelWidth::Max(1000.0), LabelWidth::Max(60.0), LabelWidth::Fixed(60.0)]
    {
        let options = LabelOptions { width, wrap: false, ..LabelOptions::default() };
        let label = engine.compile(source, &options).unwrap();
        // The same lines, glyph for glyph: an overfull line is neither wrapped nor squeezed.
        assert!(same_glyphs(&placed_glyphs(&label), &placed_glyphs(&auto)), "{width:?}");
        assert_eq!(label.semantic_text, auto.semantic_text, "{width:?}");
        assert!(!label.flags.truncated, "{width:?}");
        let expected = match width {
            LabelWidth::Max(max) => max.min(auto.metrics.width),
            LabelWidth::Fixed(fixed) => fixed,
            LabelWidth::Auto => unreachable!(),
        };
        assert_eq!(label.metrics.width, expected, "{width:?}");
        assert_eq!(engine.measure(source, &options).unwrap(), label.metrics, "{width:?}");
    }
    // Lines align within a fixed width as usual, so a centered overfull line overflows both
    // sides.
    let options = LabelOptions {
        width: LabelWidth::Fixed(60.0),
        wrap: false,
        align: LabelAlign::Center,
        ..LabelOptions::default()
    };
    let metrics = engine.measure("a \\ Revenue by region", &options).unwrap();
    let short = metrics.lines[0];
    assert_metrics_close((short.left + short.right) / 2.0, 30.0);
    let overfull = metrics.lines[1];
    assert!(overfull.left < 0.0 && overfull.right > 60.0, "{overfull:?}");
}

/// Each line is as tall as its own content, and the leading lies between one line's bottom and
/// the next line's top, so a line with tall math moves the lines after it.
#[test]
fn metrics_place_each_line() {
    let engine = engine();
    let options = LabelOptions::default();
    let plain = engine
        .measure("Revenue \\ (millions) \\ by region", &options)
        .unwrap();
    let source = "Revenue \\ ratio $display(sum_(i=1)^n x_i)$ \\ by region";
    let math = engine.measure(source, &options).unwrap();
    let leading = 0.65 * options.text.font_size;
    let height = |line: &LineMetrics| line.bottom - line.top;
    for metrics in [&plain, &math] {
        let lines = &metrics.lines;
        assert_eq!(
            (lines.len(), lines[0].top, lines[2].bottom),
            (3, 0.0, metrics.height)
        );
        for pair in lines.windows(2) {
            assert_metrics_close(pair[1].top - pair[0].bottom, leading);
        }
        for line in lines {
            assert!(line.top < line.baseline && line.baseline <= line.bottom, "{line:?}");
        }
    }
    // Only the line with the sum grows, so its baselines are no longer evenly spaced.
    assert_eq!(math.lines[0], plain.lines[0]);
    assert!(height(&math.lines[1]) > height(&plain.lines[1]) + 1.0);
    assert_metrics_close(height(&math.lines[2]), height(&plain.lines[2]));
    let pitch = |metrics: &LabelMetrics, i: usize| {
        metrics.lines[i + 1].baseline - metrics.lines[i].baseline
    };
    assert_metrics_close(pitch(&plain, 0), pitch(&plain, 1));
    assert!(pitch(&math, 0) + 1.0 < pitch(&math, 1));
}

/// A line height spaces baselines evenly, whatever the lines contain: a fixed distance, or a
/// multiple of plain lines' spacing, which keeps a line with math on the grid of plain text.
/// Lines keep their own tops and bottoms and can overlap, and the label spans them all.
#[test]
fn line_heights_space_baselines_evenly() {
    let engine = engine();
    let options = |line_height| LabelOptions { line_height, ..LabelOptions::default() };
    let measure = |source: &str, line_height| {
        engine.measure(source, &options(line_height)).unwrap()
    };
    let pitches = |metrics: &LabelMetrics| -> Vec<f32> {
        let lines = &metrics.lines;
        lines
            .windows(2)
            .map(|pair| pair[1].baseline - pair[0].baseline)
            .collect()
    };
    let span = |metrics: &LabelMetrics| {
        let lines = &metrics.lines;
        let top = lines.iter().map(|line| line.top).fold(f32::INFINITY, f32::min);
        let bottom =
            lines.iter().map(|line| line.bottom).fold(f32::NEG_INFINITY, f32::max);
        (top, bottom)
    };
    let plain = "Revenue \\ (millions) \\ by region";
    let math = "Revenue \\ ratio $display(sum_(i=1)^n x_i)$ \\ by region";
    let auto = measure(plain, LabelLineHeight::Auto);

    // A fixed distance, even one tighter than the lines, which then overlap.
    for distance in [30.0, 2.0] {
        for source in [plain, math] {
            let metrics = measure(source, LabelLineHeight::Fixed(distance));
            for pitch in pitches(&metrics) {
                assert_metrics_close(pitch, distance);
            }
            assert_eq!(span(&metrics), (0.0, metrics.height), "{source} at {distance}");
            assert_eq!(metrics.line_pitch, distance);
        }
    }

    // With Typst's spacing, plain lines' pitch is the label's line pitch, and a multiple of it
    // reproduces that spacing at 1.0, where the line with math stays on the grid.
    assert_metrics_close(pitches(&auto)[0], auto.line_pitch);
    let relative = measure(plain, LabelLineHeight::Relative(1.0));
    assert_metrics_close(relative.line_pitch, auto.line_pitch);
    for (line, auto) in relative.lines.iter().zip(&auto.lines) {
        assert_metrics_close(line.top, auto.top);
        assert_metrics_close(line.baseline, auto.baseline);
        assert_metrics_close(line.bottom, auto.bottom);
    }
    for multiple in [1.0, 1.5] {
        let metrics = measure(math, LabelLineHeight::Relative(multiple));
        for pitch in pitches(&metrics) {
            assert_metrics_close(pitch, multiple * auto.line_pitch);
        }
        assert_metrics_close(metrics.line_pitch, multiple * auto.line_pitch);
    }

    // Lines keep their glyphs' places across, and a single line is unchanged.
    let across = |label: &CompiledLabel| -> Vec<(u16, f32)> {
        placed_glyphs(label).iter().map(|glyph| (glyph.0, glyph.1)).collect()
    };
    let spaced = engine.compile(plain, &options(LabelLineHeight::Fixed(30.0))).unwrap();
    let typst = engine.compile(plain, &LabelOptions::default()).unwrap();
    assert_eq!(across(&spaced), across(&typst));
    let single = measure("Revenue", LabelLineHeight::Fixed(30.0));
    let typst = measure("Revenue", LabelLineHeight::Auto);
    assert_eq!(
        (single.width, single.height, &single.lines),
        (typst.width, typst.height, &typst.lines)
    );

    // A distance or a multiple must be finite and not negative.
    for line_height in [
        LabelLineHeight::Fixed(-1.0),
        LabelLineHeight::Fixed(f32::NAN),
        LabelLineHeight::Relative(f32::INFINITY),
    ] {
        let error = engine.compile("a", &options(line_height)).unwrap_err();
        assert!(matches!(error, LabelError::InvalidLineHeight { .. }), "{error:?}");
    }
}

/// A line spans what it draws: where alignment put it, past the label's width when a word
/// overflows, and at the alignment's position when it is empty.
#[test]
fn lines_span_what_they_draw() {
    let engine = engine();
    let measure = |source: &str, width, align, dir| {
        let mut options = LabelOptions { width, align, ..LabelOptions::default() };
        options.text.dir = dir;
        engine.measure(source, &options).unwrap()
    };
    let source = "Revenue by region in millions of dollars";
    let fixed = LabelWidth::Fixed(150.0);
    let span = |line: &LineMetrics| line.right - line.left;
    let start = measure(source, fixed, LabelAlign::Start, TextDir::Ltr).lines;
    for align in
        [LabelAlign::Left, LabelAlign::Center, LabelAlign::Right, LabelAlign::End]
    {
        let lines = measure(source, fixed, align, TextDir::Ltr).lines;
        for (line, start) in lines.iter().zip(&start) {
            // How far the line is from where its alignment puts it.
            let offset = match align {
                LabelAlign::Center => (line.left + line.right) / 2.0 - 75.0,
                LabelAlign::Right | LabelAlign::End => line.right - 150.0,
                _ => line.left,
            };
            assert_metrics_close(offset, 0.0);
            assert_metrics_close(span(line), span(start));
            assert_eq!(vertical(line), vertical(start));
        }
    }
    // In right-to-left text, start is right and end is left.
    let hebrew = "שלום עולם זה טקסט ארוך מאוד";
    for line in measure(hebrew, fixed, LabelAlign::Start, TextDir::Rtl).lines {
        assert_metrics_close(line.right, 150.0);
    }
    for line in measure(hebrew, fixed, LabelAlign::End, TextDir::Rtl).lines {
        assert_metrics_close(line.left, 0.0);
    }
    // A maximum width is the widest line's.
    let max = measure(source, LabelWidth::Max(150.0), LabelAlign::Start, TextDir::Ltr);
    let widest = max.lines.iter().map(|line| line.right).fold(0.0, f32::max);
    assert_metrics_close(widest, max.width);
    // An overfull word overflows both sides when centered.
    let fixed = LabelWidth::Fixed(40.0);
    let line =
        measure("Internationalization", fixed, LabelAlign::Center, TextDir::Ltr).lines[0];
    assert!(line.left < 0.0 && line.right > 40.0, "{line:?}");
    assert_metrics_close((line.left + line.right) / 2.0, 20.0);
    // An empty line lies where content would, and a justified one fills the width.
    let label =
        measure("a \\ \\ bbb", LabelWidth::Auto, LabelAlign::Center, TextDir::Ltr);
    let empty = label.lines[1];
    assert_eq!(empty.left, empty.right);
    assert_metrics_close(empty.left, label.width / 2.0);
    let source = "Revenue by #linebreak(justify: true) region";
    let fixed = LabelWidth::Fixed(120.0);
    let line = measure(source, fixed, LabelAlign::Start, TextDir::Ltr).lines[0];
    assert_metrics_close(line.left, 0.0);
    assert_metrics_close(line.right, 120.0);
}

/// With hanging signs, a sign that starts a line hangs out of it by its full width, so lines of
/// the same number share their left and right whatever their sign, in every alignment and
/// width. The signs still draw, outside the label's width.
#[test]
fn hanging_signs_align_numbers() {
    let engine = engine();
    let compile =
        |source: &str, options: &LabelOptions| engine.compile(source, options).unwrap();
    let span = |line: &LineMetrics| line.right - line.left;
    let numbers = "1,234.5 \\ −1,234.5 \\ +1,234.5 \\ \\-1,234.5 \\ ±1,234.5 \\ ∓1,234.5";
    for (source, dir, count) in [
        // Each sign, with the hyphen-minus of data and of some locales' formats.
        (numbers, TextDir::Ltr, 6),
        // The sign of scientific notation, which is math.
        ("#numfmt(12345.0, \".1e\") \\ #numfmt(-12345.0, \".1e\")", TextDir::Ltr, 2),
        // A right-to-left line starts at its right.
        ("10 \\ −10 \\ \\-10 \\ +10", TextDir::Rtl, 4),
    ] {
        for align in [LabelAlign::Start, LabelAlign::Center, LabelAlign::End] {
            for width in
                [LabelWidth::Auto, LabelWidth::Max(100.0), LabelWidth::Fixed(100.0)]
            {
                let mut options =
                    LabelOptions { width, align, ..LabelOptions::default() };
                options.text.dir = dir;
                options.hanging_signs = true;
                let lines = compile(source, &options).metrics.lines;
                assert_eq!(lines.len(), count, "{source}");
                for line in &lines {
                    assert_metrics_close(line.left, lines[0].left);
                    assert_metrics_close(line.right, lines[0].right);
                }
                // Without hanging signs, the signs widen their lines.
                options.hanging_signs = false;
                let pushed = compile(source, &options).metrics.lines;
                assert!(pushed.iter().all(|line| span(line) >= span(&pushed[0])));
                assert!(pushed.iter().any(|line| span(line) > span(&pushed[0]) + 1.0));
            }
        }
    }

    // The label is as wide as the number without a sign, and the signs draw left of it.
    let options = LabelOptions { hanging_signs: true, ..LabelOptions::default() };
    let label = compile(numbers, &options);
    assert_metrics_close(label.metrics.width, compile("1,234.5", &options).metrics.width);
    let mut leftmost = f32::INFINITY;
    label.frame.visit(Default::default(), &mut |ts, item| {
        if let FrameItem::Text(_) = item {
            leftmost = leftmost.min(ts.tx);
        }
    });
    assert!(leftmost < 0.0, "{leftmost}");
    // A sign that is all its line holds doesn't hang.
    let lone = compile("−", &options).metrics;
    assert_eq!(
        (lone.lines[0].left, lone.width),
        (0.0, compile("−", &LabelOptions::default()).metrics.width)
    );
}

/// Options with a width and a line limit.
fn limited(width: LabelWidth, max_lines: usize, ellipsis: bool) -> LabelOptions {
    LabelOptions {
        width,
        max_lines: NonZeroUsize::new(max_lines),
        ellipsis,
        ..LabelOptions::default()
    }
}

/// A line limit keeps a label's first lines as they are, and reports the cut.
#[test]
fn line_limits_drop_lines() {
    let engine = engine();
    let source = "Revenue by region in millions of dollars";
    let all = engine
        .compile(source, &limited(LabelWidth::Max(90.0), 0, false))
        .unwrap();
    let two = engine
        .compile(source, &limited(LabelWidth::Max(90.0), 2, false))
        .unwrap();
    assert!(two.flags.truncated && !all.flags.truncated);
    assert!(two.metrics.height < all.metrics.height);
    assert!(all.semantic_text.starts_with(two.semantic_text.trim_end()));
    let kept = placed_glyphs(&two);
    assert!(same_glyphs(&kept, &placed_glyphs(&all)[..kept.len()]));
    // A limit the label is within cuts nothing.
    let three = engine
        .compile(source, &limited(LabelWidth::Max(90.0), 3, true))
        .unwrap();
    assert!(!three.flags.truncated);
    assert!(same_glyphs(&placed_glyphs(&three), &placed_glyphs(&all)));
    // Explicit breaks count too.
    let label = engine
        .compile("a \\ b \\ c", &limited(LabelWidth::Auto, 2, false))
        .unwrap();
    assert_eq!((label.semantic_text.as_str(), label.flags.truncated), ("a\nb", true));
    // Dropped lines that show nothing cut nothing.
    let label = engine
        .compile("a \\ b \\ \\ ", &limited(LabelWidth::Auto, 2, true))
        .unwrap();
    assert_eq!((label.semantic_text.as_str(), label.flags.truncated), ("a\nb", false));
}

/// An ellipsis ends the last line where text is cut, which is shortened at grapheme
/// boundaries to fit it. The kept text lays out as it does alone, and the ellipsis follows it
/// unkerned, as an inserted hyphen does.
#[test]
fn ellipses_end_cut_text() {
    let engine = engine();
    for (source, width, max_lines, visible) in [
        (
            "Revenue by region in millions of dollars",
            90.0,
            2,
            "Revenue by region in millio…",
        ),
        ("Revenue by region in millions of dollars", 90.0, 1, "Revenue by…"),
        // Without dropped lines, an overfull line is cut.
        ("Internationalization", 40.0, 0, "Intern…"),
        // A soft hyphen shows no hyphen before the ellipsis.
        ("Inter-?national-?ization", 60.0, 1, "Inter…"),
    ] {
        let options = limited(LabelWidth::Max(width), max_lines, true);
        let label = engine.compile(source, &options).unwrap();
        assert_eq!(label.semantic_text, visible, "{source}");
        assert_eq!(label.flags.truncated, visible.ends_with('…'), "{source}");
        let unlimited = LabelOptions { max_lines: None, ellipsis: false, ..options };
        let kept = visible.trim_end_matches('…');
        let alone = placed_glyphs(&engine.compile_text(kept, &unlimited).unwrap());
        let mut glyphs = placed_glyphs(&label);
        if label.flags.truncated {
            let (_, x, y, _) = glyphs.pop().unwrap();
            let &(_, last_x, last_y, advance) = alone.last().unwrap();
            assert!((x - (last_x + advance)).abs() < 0.01 && y == last_y, "{source}");
            assert!(label.metrics.width <= width, "{source}");
        }
        assert!(same_glyphs(&glyphs, &alone), "{source}");
    }
}

/// With an ellipsis, every line wider than the width is cut to fit, whether lines wrap or end
/// only at explicit breaks. A cut wrapped line keeps the space before the next line's text, an
/// explicit break keeps its newline, and a hanging sign lies outside the width.
#[test]
fn ellipses_cut_every_overfull_line() {
    let engine = engine();
    let fits = |label: &CompiledLabel, width: f32| {
        let lines = &label.metrics.lines;
        lines.iter().all(|line| line.right - line.left <= width + 0.01)
    };
    // A wrapped word wider than the width is cut, and the lines after it stay.
    let options = limited(LabelWidth::Max(40.0), 0, true);
    let label = engine.compile("Internationalization of labels", &options).unwrap();
    assert!(label.flags.truncated && fits(&label, 40.0));
    assert!(label.semantic_text.starts_with("Intern… of"), "{}", label.semantic_text);
    assert!(label.semantic_text.ends_with("labels"), "{}", label.semantic_text);

    // Unwrapped lines are each cut, and lines within the width stay whole.
    let options = LabelOptions {
        wrap: false,
        ..limited(LabelWidth::Max(60.0), 0, true)
    };
    let source = "Revenue by region \\ in millions of dollars \\ USD";
    let label = engine.compile(source, &options).unwrap();
    let lines: Vec<_> = label.semantic_text.split('\n').collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].ends_with('…') && lines[1].ends_with('…'), "{lines:?}");
    assert_eq!(lines[2], "USD");
    assert!(label.flags.truncated && fits(&label, 60.0));
    // A cut line lays out as its kept text does alone, followed by the ellipsis.
    let kept = lines[0].trim_end_matches('…');
    let alone = engine.compile_text(kept, &LabelOptions::default()).unwrap();
    let first = placed_glyphs(&label);
    let alone = placed_glyphs(&alone);
    assert!(same_glyphs(&first[..alone.len()], &alone), "{kept}");

    // In right-to-left text, each cut line's ellipsis is its leftmost glyph.
    let mut options = LabelOptions {
        wrap: false,
        ..limited(LabelWidth::Max(50.0), 0, true)
    };
    options.text.dir = TextDir::Rtl;
    let label = engine
        .compile("שלום עולם זה טקסט \\ ארוך מאוד מאוד", &options)
        .unwrap();
    let mut items = vec![];
    label.frame.visit(Default::default(), &mut |ts, item| {
        if let FrameItem::Text(text) = item {
            items.push((ts.ty, ts.tx, text.text == "…"));
        }
    });
    for line in &label.metrics.lines {
        let leftmost = items
            .iter()
            .filter(|(y, ..)| (y - line.baseline).abs() < 0.01)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        assert!(leftmost.2, "{items:?}");
    }

    // A line that fits without its hanging sign isn't cut.
    let digits = engine.measure("1,234.5", &LabelOptions::default()).unwrap().width;
    let mut options = LabelOptions {
        wrap: false,
        hanging_signs: true,
        ..limited(LabelWidth::Max(digits), 0, true)
    };
    let source = "−1,234.5 \\ +1,234.5";
    let label = engine.compile(source, &options).unwrap();
    assert!(!label.flags.truncated, "{}", label.semantic_text);
    options.hanging_signs = false;
    assert!(engine.compile(source, &options).unwrap().flags.truncated);
}

/// The edges of ellipses: whole clusters, explicit breaks, equations, right-to-left text and
/// widths too narrow for the ellipsis.
#[test]
fn ellipses_keep_clusters_and_directions() {
    let engine = engine();
    // A cut never splits a grapheme cluster.
    for source in [
        "Family 👨\u{200d}👩\u{200d}👧 emoji",
        "e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}",
    ] {
        for width in (2..60).map(|width| width as f32) {
            let options = limited(LabelWidth::Max(width), 1, true);
            let label = engine.compile_text(source, &options).unwrap();
            let kept = label.semantic_text.trim_end_matches('…');
            let ends = source.grapheme_indices(true).map(|(i, g)| i + g.len());
            assert!(
                kept.is_empty() || ends.into_iter().any(|end| end == kept.len()),
                "{kept:?}"
            );
        }
    }
    // Without a width, the ellipsis follows the last kept line.
    let label = engine
        .compile("a \\ b \\ c", &limited(LabelWidth::Auto, 2, true))
        .unwrap();
    assert_eq!(label.semantic_text, "a\nb…");
    // Equations are cut between their pieces.
    let options = limited(LabelWidth::Max(60.0), 1, true);
    let label = engine.compile("Fit $y = 2.5 x + 7$ to the data", &options).unwrap();
    assert_eq!(label.semantic_text, "Fit 𝑦=…");
    // A width narrower than the ellipsis leaves the ellipsis alone.
    let label = engine
        .compile("Revenue", &limited(LabelWidth::Max(2.0), 1, true))
        .unwrap();
    assert_eq!((label.semantic_text.as_str(), label.flags.truncated), ("…", true));
    // In right-to-left text, the ellipsis is the leftmost glyph.
    let mut options = limited(LabelWidth::Max(60.0), 1, true);
    options.text.dir = TextDir::Rtl;
    let label = engine.compile("שלום עולם זה טקסט ארוך מאוד", &options).unwrap();
    assert!(label.semantic_text.ends_with('…'));
    let mut xs = vec![];
    label.frame.visit(Default::default(), &mut |ts, item| {
        if let FrameItem::Text(text) = item {
            xs.push((ts.tx, text.text == "…"));
        }
    });
    xs.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert!(xs[0].1, "{xs:?}");
}

#[test]
fn empty_labels_are_empty() {
    let engine = engine();
    let options = LabelOptions::default();
    for label in [engine.compile("", &options), engine.compile_text("", &options)] {
        let label = label.unwrap();
        assert_eq!((label.metrics.width, label.metrics.height), (0.0, 0.0));
        assert_eq!(label.metrics.lines.len(), 1);
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
    // As native text, the emoji is a run too, and its image names the run's text item.
    let svg = svg_items(&label, &SvgOptions { native_text: true });
    let run = svg
        .items
        .iter()
        .find_map(|item| match item {
            SvgItem::Text(run) if run.text == family => Some(run.text_item),
            _ => None,
        })
        .expect("the emoji is a run");
    assert!(svg.items.iter().any(|item| matches!(
        item,
        SvgItem::Image(image) if image.glyph.text == run
    )));
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
