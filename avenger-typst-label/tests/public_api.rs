mod common;

use avenger_typst_label::{
    Color, CompiledLabel, EngineOptions, FontStyle, FontWeight, LabelEngine, LabelError,
    LabelFrameItem, LabelOptions, LabelParamValue, MathFontSpec, PdfDrawItem, PdfOptions,
    SvgOptions, TextItemKind, escape_text, pdf_items, svg_items,
};
use indexmap::IndexMap;
use std::path::{Path, PathBuf};

#[cfg(feature = "raster")]
use avenger_typst_label::{RasterOptions, rasterize};

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.001,
        "expected {actual} to be within 0.001 of {expected}"
    );
}

fn has_text_kind(label: &CompiledLabel, kind: TextItemKind) -> bool {
    label.frame.items.iter().any(|(_, item)| match item {
        LabelFrameItem::Text(text) => text.kind == kind,
        LabelFrameItem::Group(group) => group
            .items
            .iter()
            .any(|(_, item)| matches!(item, LabelFrameItem::Text(text) if text.kind == kind)),
        LabelFrameItem::Shape(_) | LabelFrameItem::Image(_) => false,
    })
}

fn has_shape(label: &CompiledLabel) -> bool {
    label.frame.items.iter().any(|(_, item)| match item {
        LabelFrameItem::Shape(_) => true,
        LabelFrameItem::Group(group) => group
            .items
            .iter()
            .any(|(_, item)| matches!(item, LabelFrameItem::Shape(_))),
        LabelFrameItem::Text(_) | LabelFrameItem::Image(_) => false,
    })
}

#[test]
fn final_public_api_compiles_measures_and_lowers_markup_label() {
    let engine = LabelEngine::new(common::engine_options()).unwrap();
    let mut options = LabelOptions::default();
    options.params.insert(
        "series_name".to_string(),
        LabelParamValue::Str("Revenue".to_string()),
    );

    let source = "#strong[#series_name] $sqrt(x^2 + y^2)$";
    let label = engine.compile(source, &options).unwrap();
    let measured = engine.measure(source, &options).unwrap();

    assert!(label.flags.has_markup);
    assert!(label.flags.has_math);
    assert!(label.metrics.width > 0.0);
    assert!(label.metrics.height > 0.0);
    assert_close(measured.width, label.metrics.width);
    assert_close(measured.height, label.metrics.height);
    assert!(has_text_kind(&label, TextItemKind::Plain));
    assert!(has_text_kind(&label, TextItemKind::Math));
    assert!(has_shape(&label), "sqrt should emit a radical shape");

    let svg = svg_items(&label, &SvgOptions::default()).unwrap();
    assert_eq!(svg.metrics, label.metrics);
    assert!(!svg.items.is_empty());
    assert!(!svg.font_resources.is_empty());

    let pdf = pdf_items(&label, &PdfOptions::default()).unwrap();
    assert_eq!(pdf.metrics, label.metrics);
    assert!(pdf.semantic_text.contains("Revenue"));
    assert!(!pdf.glyph_runs.is_empty());
    assert!(
        pdf.draw_items
            .iter()
            .any(|item| matches!(item, PdfDrawItem::GlyphRun(_)))
    );
    assert!(
        pdf.draw_items
            .iter()
            .any(|item| matches!(item, PdfDrawItem::PathItem(_)))
    );
}

#[test]
fn final_public_api_exposes_options_and_external_param_model() {
    let mut engine_options = EngineOptions::default();
    engine_options.fonts.load_system_fonts = false;
    engine_options
        .fonts
        .extra_font_families
        .push("Lato".to_string());

    assert!(!engine_options.fonts.load_system_fonts);
    assert_eq!(engine_options.fonts.extra_font_families, ["Lato"]);

    let mut dict = IndexMap::new();
    dict.insert("paint".to_string(), LabelParamValue::Str("red".to_string()));
    let mut options = LabelOptions::default();
    options.text.font_family = "Lato".to_string();
    options.text.font_size = 15.0;
    options.text.fill = Color::rgba(0.1, 0.2, 0.3, 1.0);
    options.text.font_weight = FontWeight::Number(500);
    options.text.font_style = FontStyle::Italic;
    options.math.font = MathFontSpec::LeteSansMath;
    options.math.font_size = 15.0;
    options.math.fill = Color::rgba(0.3, 0.2, 0.1, 1.0);
    options.math.font_weight = FontWeight::Bold;
    options
        .params
        .insert("none".to_string(), LabelParamValue::None);
    options
        .params
        .insert("flag".to_string(), LabelParamValue::Bool(true));
    options
        .params
        .insert("count".to_string(), LabelParamValue::Int(7));
    options
        .params
        .insert("ratio".to_string(), LabelParamValue::Float(0.25));
    options.params.insert(
        "name".to_string(),
        LabelParamValue::Str("Series".to_string()),
    );
    options.params.insert(
        "array".to_string(),
        LabelParamValue::Array(vec![LabelParamValue::Int(1)]),
    );
    options
        .params
        .insert("stroke".to_string(), LabelParamValue::Dict(dict));
    options.limits.max_source_bytes = 4;

    let engine = LabelEngine::new(common::engine_options()).unwrap();
    let err = engine.compile("12345", &options).unwrap_err();
    assert!(matches!(
        err,
        LabelError::SourceTooLarge {
            actual: 5,
            limit: 4
        }
    ));
}

#[test]
fn output_lowerers_consume_compiled_frame_not_source_text() {
    let engine = LabelEngine::new(common::engine_options()).unwrap();
    let mut label = engine
        .compile("Price \\$7 $sqrt(x)$", &LabelOptions::default())
        .unwrap();
    let metrics = label.metrics;

    label.source = "this would be invalid if a lowerer parsed it: $x^$ #let".to_string();

    let svg = svg_items(&label, &SvgOptions::default()).unwrap();
    assert_eq!(svg.metrics, metrics);
    assert!(!svg.items.is_empty());

    let pdf = pdf_items(&label, &PdfOptions::default()).unwrap();
    assert_eq!(pdf.metrics, metrics);
    assert!(pdf.semantic_text.contains("Price $7"));
    assert!(!pdf.draw_items.is_empty());
}

#[cfg(feature = "raster")]
#[test]
fn raster_lowerer_consumes_compiled_frame_not_source_text() {
    let engine = LabelEngine::new(common::engine_options()).unwrap();
    let mut label = engine
        .compile("Price \\$7 $sqrt(x)$", &LabelOptions::default())
        .unwrap();

    label.source = "this would be invalid if raster parsed it: $x^$ #let".to_string();

    let raster = rasterize(&label, &RasterOptions { scale: 1.0 }).unwrap();
    assert!(raster.image.width > 0);
    assert!(raster.image.height > 0);
}

#[test]
fn final_public_api_literal_fast_path_matches_escaped_markup() {
    let engine = LabelEngine::new(common::engine_options()).unwrap();
    let options = LabelOptions::default();
    let text = "cost $5 #literal [brackets] *stars* http://example.com 🚀 שלום नमस्ते";

    let literal = engine.compile_text(text, &options).unwrap();
    let escaped = engine.compile(&escape_text(text), &options).unwrap();
    let measured = engine.measure_text(text, &options).unwrap();

    assert!(!literal.flags.has_markup);
    assert!(!literal.flags.has_math);
    assert_eq!(literal.semantic_text(), text);
    assert_close(literal.metrics.width, escaped.metrics.width);
    assert_close(literal.metrics.height, escaped.metrics.height);
    assert_close(measured.width, literal.metrics.width);
    assert_close(measured.height, literal.metrics.height);
}

#[test]
fn final_public_api_extracts_referenced_params() {
    let source = "#upper[#series] #underline(stroke: series_color)[care] \
        $y = #slope x + #intercept$ #series";
    let expected = vec![
        "series".to_string(),
        "series_color".to_string(),
        "slope".to_string(),
        "intercept".to_string(),
    ];

    assert_eq!(
        avenger_typst_label::referenced_params(source).unwrap(),
        expected
    );

    let engine = LabelEngine::new(common::engine_options()).unwrap();
    assert_eq!(engine.referenced_params(source).unwrap(), expected);
}

#[test]
fn typst_mirrored_modules_do_not_depend_on_public_label_params() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");
    let mut sources = Vec::new();
    collect_typst_sources(&src_dir, &mut sources);

    let forbidden = ["LabelParams", "LabelParamValue", "render_label_param"];
    let mut matches = Vec::new();
    for path in sources {
        let text = std::fs::read_to_string(&path).unwrap();
        for (index, line) in text.lines().enumerate() {
            if forbidden.iter().any(|needle| line.contains(needle)) {
                matches.push(format!("{}:{}: {line}", path.display(), index + 1));
            }
        }
    }

    assert!(
        matches.is_empty(),
        "mirrored typst modules should use typst_library::foundations::Scope, not public label params:\n{}",
        matches.join("\n")
    );
}

fn collect_typst_sources(dir: &Path, sources: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            let is_typst_dir = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "typst_library" || name.starts_with("typst_"));
            if is_typst_dir {
                collect_rs_sources(&path, sources);
            }
        }
    }
}

fn collect_rs_sources(dir: &Path, sources: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

#[test]
fn missing_font_policy_errors_warns_or_falls_back() {
    use avenger_typst_label::{LabelWarning, MissingFontPolicy};
    let mut options = LabelOptions::default();
    options.text.font_family = "UnavailableFontForPolicyTest".to_string();
    for policy in [
        MissingFontPolicy::Error,
        MissingFontPolicy::Warn,
        MissingFontPolicy::Fallback,
    ] {
        let mut engine_options = common::engine_options();
        engine_options.fonts.load_system_fonts = false;
        engine_options.fonts.missing_font = policy;
        let engine = LabelEngine::new(engine_options).unwrap();
        let result = engine.compile_text("Text", &options);
        match policy {
            MissingFontPolicy::Error => {
                assert!(matches!(result, Err(LabelError::MissingFont { .. })))
            }
            MissingFontPolicy::Warn => assert!(
                result
                    .unwrap()
                    .warnings
                    .iter()
                    .any(|warning| matches!(warning, LabelWarning::MissingFont { .. }))
            ),
            MissingFontPolicy::Fallback => assert!(result.unwrap().warnings.is_empty()),
        }
    }
}
