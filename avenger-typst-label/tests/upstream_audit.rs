mod common;

use avenger_typst_label::{
    CompiledLabel, FontWeight, LabelEngine, LabelFrameItem, LabelOptions, PdfGlyph, PdfOptions,
    TextItem, pdf_items,
};

fn engine() -> LabelEngine {
    let mut options = common::engine_options();
    options.fonts.load_system_fonts = false;
    LabelEngine::new(options).unwrap()
}

fn options() -> LabelOptions {
    let mut options = LabelOptions::default();
    options.text.font_family = "Lato".into();
    options.text.font_size = 32.0;
    options.text.font_weight = FontWeight::Number(500);
    options.math.font_size = 32.0;
    options.math.font_weight = FontWeight::Number(500);
    options
}

fn texts(label: &CompiledLabel) -> Vec<&TextItem> {
    label
        .frame
        .items
        .iter()
        .filter_map(|(_, item)| match item {
            LabelFrameItem::Text(text) => Some(text),
            _ => None,
        })
        .collect()
}

fn glyphs(label: &CompiledLabel) -> Vec<PdfGlyph> {
    pdf_items(label, &PdfOptions::default())
        .unwrap()
        .glyph_runs
        .into_iter()
        .flat_map(|run| run.glyphs)
        .collect()
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
        assert_eq!(glyphs(&a), glyphs(&b));
    }
}

#[test]
fn scripted_rows_share_a_baseline() {
    let label = engine().compile("$x + script(y+z)$", &options()).unwrap();
    let glyphs = glyphs(&label);
    let baseline = glyphs
        .iter()
        .find(|g| g.unicode == "𝑥")
        .unwrap()
        .transform
        .ty;
    for glyph in glyphs
        .iter()
        .filter(|g| ["𝑦", "𝑧"].contains(&g.unicode.as_str()))
    {
        assert!((glyph.transform.ty - baseline).abs() < 0.001);
    }
}

#[test]
fn script_features_require_coverage_of_spaces() {
    let engine = engine();
    for kind in ["sub", "super"] {
        let automatic = engine
            .compile(&format!("H#{kind}[2 2]O"), &options())
            .unwrap();
        let synthetic = engine
            .compile(&format!("H#{kind}(typographic: false)[2 2]O"), &options())
            .unwrap();
        assert_eq!(glyphs(&automatic), glyphs(&synthetic));
        assert!(
            texts(&automatic)
                .iter()
                .all(|text| text.font_features.is_empty())
        );
        let single = engine
            .compile(&format!("H#{kind}[2]O"), &options())
            .unwrap();
        let text = texts(&single).into_iter().find(|t| t.text == "2").unwrap();
        assert_eq!(text.style.as_ref().unwrap().font_size, 32.0);
        assert_eq!(text.font_features.len(), 1);
    }
}

#[test]
fn synthetic_scripts_keep_subpoint_sizes_and_missing_metrics_defaults() {
    let engine = engine();
    let small = engine
        .compile("H#sub(typographic: false, size: 0.2pt)[2]O", &options())
        .unwrap();
    let script = texts(&small).into_iter().find(|t| t.text == "2").unwrap();
    assert!((script.style.as_ref().unwrap().font_size - 0.2).abs() < 0.0001);
    let mut tiny = options();
    tiny.text.font_size = 0.5;
    let tiny = engine.compile_text("Hello", &tiny).unwrap();
    let full = engine.compile_text("Hello", &options()).unwrap();
    assert!((tiny.metrics.width * 64.0 - full.metrics.width).abs() < 0.001);
    let mut missing = options();
    missing.text.font_family = "AuditNoScriptMetrics".into();
    let label = engine
        .compile("H#super(typographic: false)[2]O", &missing)
        .unwrap();
    let script = texts(&label).into_iter().find(|t| t.text == "2").unwrap();
    assert!((script.style.as_ref().unwrap().font_size - 19.2).abs() < 0.001);
    let gs = glyphs(&label);
    assert!((gs[0].transform.ty - gs[1].transform.ty - 16.0).abs() < 0.001);
}

#[test]
fn bidi_across_markup_keeps_logical_text_and_glyph_positions() {
    let engine = engine();
    for (plain, decorated) in [
        ("abc אבג 123 xyz", "abc #underline[אבג 123] xyz"),
        ("אבג דהו", "אבג #underline[דהו]"),
    ] {
        let plain_label = engine.compile(plain, &options()).unwrap();
        let label = engine.compile(decorated, &options()).unwrap();
        assert_eq!(label.semantic_text(), plain);
        let positions = |label: &CompiledLabel| {
            glyphs(label)
                .into_iter()
                .map(|g| (g.glyph_id, g.transform.tx, g.transform.ty))
                .collect::<Vec<_>>()
        };
        let a = positions(&plain_label);
        let b = positions(&label);
        assert_eq!(a.len(), b.len());
        for ((ag, ax, ay), (bg, bx, by)) in a.into_iter().zip(b) {
            assert_eq!(ag, bg);
            assert!((ax - bx).abs() < 0.001);
            assert!((ay - by).abs() < 0.001);
        }
    }
}

#[test]
fn complex_math_text_retains_fallback_fonts_and_cluster_ranges() {
    let label = engine()
        .compile("$\"हिन्दी\" \"אבג 123\"$", &options())
        .unwrap();
    let pdf = pdf_items(&label, &PdfOptions::default()).unwrap();
    assert!(
        pdf.font_resources
            .iter()
            .any(|f| f.family == "Noto Sans Devanagari")
    );
    assert!(
        pdf.font_resources
            .iter()
            .any(|f| f.family == "Noto Sans Hebrew")
    );
    for run in pdf.glyph_runs {
        for glyph in run.glyphs {
            assert_ne!(glyph.glyph_id, 0);
            assert_eq!(run.text.get(glyph.text_range), Some(glyph.unicode.as_str()));
        }
    }
}

#[test]
fn variable_font_instances_keep_coordinates_in_pdf_resources() {
    let engine = engine();
    let mut options = options();
    options.text.font_family = "Noto Sans Hebrew".into();
    for weight in [400, 700] {
        options.text.font_weight = FontWeight::Number(weight);
        let label = engine.compile("אבג", &options).unwrap();
        let pdf = pdf_items(&label, &PdfOptions::default()).unwrap();
        assert_eq!(pdf.font_resources.len(), 1);
        assert!(
            pdf.font_resources[0]
                .variations
                .contains(&(*b"wght", weight as f32))
        );
    }
    let label = engine.compile("אבג #strong[דהו]", &options).unwrap();
    let pdf = pdf_items(&label, &PdfOptions::default()).unwrap();
    assert_eq!(
        pdf.font_resources.len(),
        2,
        "distinct instances must not share a resource ID"
    );
}

#[test]
fn strong_delta_saturates_at_both_integer_boundaries() {
    let engine = engine();
    for (delta, expected) in [(i64::MIN, 1), (i64::MAX, 1000)] {
        let mut options = options();
        options.params.insert(
            "amount".into(),
            avenger_typst_label::LabelParamValue::Int(delta),
        );
        let label = engine
            .compile("#strong(delta: amount)[A]", &options)
            .unwrap();
        assert_eq!(
            texts(&label)[0].style.as_ref().unwrap().font_weight,
            FontWeight::Number(expected)
        );
    }
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
    for (label, before) in [(&behind, true), (&in_front, false)] {
        let text = label
            .frame
            .items
            .iter()
            .position(|(_, item)| matches!(item, LabelFrameItem::Text(_)))
            .unwrap();
        let decoration = label
            .frame
            .items
            .iter()
            .position(
                |(_, item)| matches!(item, LabelFrameItem::Shape(s) if s.item.stroke.is_some()),
            )
            .unwrap();
        assert_eq!(decoration < text, before);
    }
    assert_ne!(
        rasterize(&behind, &RasterOptions::default())
            .unwrap()
            .image
            .data,
        rasterize(&in_front, &RasterOptions::default())
            .unwrap()
            .image
            .data
    );
}

#[test]
fn case_conversion_keeps_nested_decorations_on_utf8_boundaries() {
    let engine = engine();
    for (source, expected) in [
        ("#lower[I#strike[İ]A]", "ii\u{307}a"),
        ("#upper[a#strike[ß]c]", "ASSC"),
    ] {
        let label = engine.compile(source, &options()).unwrap();
        assert_eq!(label.semantic_text(), expected);
        assert!(
            label.frame.items.iter().any(
                |(_, item)| matches!(item, LabelFrameItem::Shape(s) if s.item.stroke.is_some())
            )
        );
    }
}
