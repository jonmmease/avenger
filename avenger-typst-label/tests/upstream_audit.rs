mod common;

use avenger_typst_label::{
    CompiledLabel, FontWeight, FrameItem, LabelEngine, LabelOptions, TextItem,
};

fn engine() -> LabelEngine {
    let mut options = common::engine_options();
    options.fonts.load_system_fonts = false;
    LabelEngine::new(options)
}

fn options() -> LabelOptions {
    let mut options = LabelOptions::default();
    options.text.font_family = "Lato".into();
    options.text.font_size = 32.0;
    options.text.font_weight = FontWeight::from_number(500);
    options.math.font_weight = Some(FontWeight::from_number(500));
    options
}

fn texts(label: &CompiledLabel) -> Vec<&TextItem> {
    label.frame.text_items().into_iter().map(|(_, text)| text).collect()
}

/// A glyph with its cluster's text and its absolute origin.
#[derive(Debug, PartialEq)]
struct PlacedGlyph {
    id: u16,
    text: String,
    x: f32,
    y: f32,
}

fn glyphs(label: &CompiledLabel) -> Vec<PlacedGlyph> {
    label
        .frame
        .text_items()
        .into_iter()
        .flat_map(|(ts, text)| {
            text.positioned_glyphs().map(move |(pos, glyph)| {
                let origin = ts.apply(pos);
                PlacedGlyph {
                    id: glyph.id,
                    text: text.text[glyph.range.clone()].to_string(),
                    x: origin.x,
                    y: origin.y,
                }
            })
        })
        .collect()
}

/// Whether two glyph lists agree within a thousandth of a point.
fn assert_same_glyphs(actual: &[PlacedGlyph], expected: &[PlacedGlyph], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}");
    for (a, e) in actual.iter().zip(expected) {
        assert_eq!((a.id, &a.text), (e.id, &e.text), "{what}");
        assert!((a.x - e.x).abs() < 0.001 && (a.y - e.y).abs() < 0.001, "{what}");
    }
}

#[test]
fn custom_operators_match_predefined_operators() {
    let engine = engine();
    for (predefined, custom) in [
        ("$sin x$", "$op(\"sin\") x$"),
        ("$script(sin x)$", "$script(op(\"sin\") x)$"),
        (
            "$display(lim_(n -> oo) x)$",
            "$display(op(\"lim\", limits: #true)_(n -> oo) x)$",
        ),
        ("$display(scripts(lim)_(n -> oo) x)$", "$display(op(\"lim\")_(n -> oo) x)$"),
    ] {
        let expected = engine.compile(predefined, &options()).unwrap();
        let actual = engine.compile(custom, &options()).unwrap();
        assert_eq!(actual.metrics, expected.metrics, "{custom}");
        assert_same_glyphs(&glyphs(&actual), &glyphs(&expected), custom);
    }
}

#[test]
fn custom_operators_preserve_body_layout() {
    let engine = engine();
    for body in ["EE", "integral", "stretch(|, size: #300%)", "frac(x, y)"] {
        let expected = engine.compile(&format!("$display({body})$"), &options()).unwrap();
        let actual =
            engine.compile(&format!("$display(op({body}))$"), &options()).unwrap();
        assert_eq!(actual.metrics, expected.metrics, "{body}");
        assert_same_glyphs(&glyphs(&actual), &glyphs(&expected), body);
    }
}

#[test]
fn absolute_math_sizes_are_idempotent_and_restore_text_size() {
    let engine = engine();
    for (a, b) in [
        ("$script(script(x))$", "$script(x)$"),
        ("$sscript(sscript(x))$", "$sscript(x)$"),
        ("$script(inline(x))$", "$x$"),
    ] {
        let a = engine.compile(a, &options()).unwrap();
        let b = engine.compile(b, &options()).unwrap();
        assert_eq!(a.metrics, b.metrics);
        assert_same_glyphs(&glyphs(&a), &glyphs(&b), "sizes");
    }
}

#[test]
fn scripted_rows_share_a_baseline() {
    let label = engine().compile("$x + script(y+z)$", &options()).unwrap();
    let glyphs = glyphs(&label);
    let baseline = glyphs.iter().find(|g| g.text == "𝑥").unwrap().y;
    for glyph in glyphs.iter().filter(|g| ["𝑦", "𝑧"].contains(&g.text.as_str())) {
        assert!((glyph.y - baseline).abs() < 0.001);
    }
}

#[test]
fn script_features_require_coverage_of_spaces() {
    let engine = engine();
    for kind in ["sub", "super"] {
        // The font's script features don't cover the space, so both synthesize.
        let automatic = engine.compile(&format!("H#{kind}[2 2]O"), &options()).unwrap();
        let synthetic = engine
            .compile(&format!("H#{kind}(typographic: false)[2 2]O"), &options())
            .unwrap();
        assert_same_glyphs(&glyphs(&automatic), &glyphs(&synthetic), kind);
        // A covered script keeps the text's size and takes the feature's glyph.
        let single = engine.compile(&format!("H#{kind}[2]O"), &options()).unwrap();
        let text = texts(&single).into_iter().find(|t| t.text == "2").unwrap();
        assert_eq!(text.size, 32.0);
        let plain = engine.compile("2", &options()).unwrap();
        assert_ne!(text.glyphs[0].id, texts(&plain)[0].glyphs[0].id, "{kind}");
    }
}

#[test]
fn synthetic_scripts_keep_subpoint_sizes_and_missing_metrics_defaults() {
    let engine = engine();
    let small = engine
        .compile("H#sub(typographic: false, size: 0.2pt)[2]O", &options())
        .unwrap();
    let script = texts(&small).into_iter().find(|t| t.text == "2").unwrap();
    assert!((script.size - 0.2).abs() < 0.0001);
    let mut tiny = options();
    tiny.text.font_size = 0.5;
    let tiny = engine.compile_text("Hello", &tiny).unwrap();
    let full = engine.compile_text("Hello", &options()).unwrap();
    assert!((tiny.metrics.width * 64.0 - full.metrics.width).abs() < 0.001);
    let mut missing = options();
    missing.text.font_family = "AuditNoScriptMetrics".into();
    let label = engine.compile("H#super(typographic: false)[2]O", &missing).unwrap();
    let script = texts(&label).into_iter().find(|t| t.text == "2").unwrap();
    assert!((script.size - 19.2).abs() < 0.001);
    let gs = glyphs(&label);
    assert!((gs[0].y - gs[1].y - 16.0).abs() < 0.001);
}

#[test]
fn complex_math_text_retains_fallback_fonts_and_cluster_ranges() {
    let label = engine().compile("$\"हिन्दी\" \"אבג 123\"$", &options()).unwrap();
    let families: Vec<_> = texts(&label).iter().map(|text| text.font.family()).collect();
    assert!(families.contains(&"Noto Sans Devanagari"), "{families:?}");
    assert!(families.contains(&"Noto Sans Hebrew"), "{families:?}");
    for text in texts(&label) {
        for glyph in &text.glyphs {
            assert_ne!(glyph.id, 0);
            assert!(text.text.get(glyph.range.clone()).is_some_and(|t| !t.is_empty()));
        }
    }
}

#[test]
fn variable_font_instances_keep_their_coordinates() {
    let engine = engine();
    let mut options = options();
    options.text.font_family = "Noto Sans Hebrew".into();
    for weight in [400, 700] {
        options.text.font_weight = FontWeight::from_number(weight);
        let label = engine.compile("אבג", &options).unwrap();
        let fonts: Vec<_> = texts(&label).iter().map(|text| text.font.clone()).collect();
        assert!(fonts.iter().all(|font| *font == fonts[0]));
        assert!(fonts[0].variations().contains(&(*b"wght", weight as f32)));
    }
    let label = engine.compile("אבג #strong[דהו]", &options).unwrap();
    let texts = texts(&label);
    assert_ne!(texts[0].font, texts[texts.len() - 1].font, "distinct instances");
}

#[test]
fn strong_delta_saturates_at_both_integer_boundaries() {
    let engine = engine();
    for (delta, expected) in [(i64::MIN, "Lato-Light"), (i64::MAX, "Lato-Bold")] {
        let values = avenger_typst_label::LabelValues::from([(
            "amount".to_string(),
            avenger_typst_label::LabelValue::Int(delta),
        )]);
        let source =
            avenger_typst_label::bind("#strong(delta: amount)[A]", &values).unwrap();
        let label = engine.compile(&source, &options()).unwrap();
        assert_eq!(texts(&label)[0].font.postscript_name().as_deref(), Some(expected));
    }
}

/// The drawing order of a label's text and stroked shapes.
fn order(label: &CompiledLabel) -> Vec<&'static str> {
    let mut order = vec![];
    label.frame.visit(Default::default(), &mut |_, item| match item {
        FrameItem::Text(_) => order.push("text"),
        FrameItem::Shape(shape) if shape.stroke.is_some() => order.push("stroke"),
        _ => {}
    });
    order
}

#[cfg(feature = "raster")]
#[test]
fn decoration_background_controls_frame_and_raster_order() {
    use avenger_typst_label::{RasterOptions, rasterize};
    let engine = engine();
    let make = |background| {
        engine.compile(&format!("#underline(evade: false, offset: -10pt, stroke: 3pt + red, background: {background})[abc]"), &options()).unwrap()
    };
    let behind = make(true);
    let in_front = make(false);
    assert_eq!(order(&behind), ["stroke", "text"]);
    assert_eq!(order(&in_front), ["text", "stroke"]);
    assert_ne!(
        rasterize(&behind, &RasterOptions::default()).unwrap().image.data,
        rasterize(&in_front, &RasterOptions::default()).unwrap().image.data
    );
}

#[test]
fn case_conversion_keeps_nested_decorations_on_utf8_boundaries() {
    let engine = engine();
    for (source, expected) in
        [("#lower[I#strike[İ]A]", "ii\u{307}a"), ("#upper[a#strike[ß]c]", "ASSC")]
    {
        let label = engine.compile(source, &options()).unwrap();
        assert_eq!(label.semantic_text, expected);
        assert!(order(&label).contains(&"stroke"), "{source}");
    }
}
