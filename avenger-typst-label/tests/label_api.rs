use avenger_typst_label::{
    LabelEngine, LabelError, LabelFrameItem, LabelOptions, LabelParamValue, PdfOptions, SvgOptions,
    TextItemKind, escape_text, pdf_items, svg_items,
};

#[cfg(feature = "raster")]
use avenger_typst_label::{RasterOptions, rasterize};

fn engine() -> LabelEngine {
    LabelEngine::new(Default::default()).unwrap()
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

#[test]
fn compile_text_plain_ascii_matches_escaped_compile() {
    assert_same_literal_rendering("Revenue by region");
}

#[test]
fn compile_text_literal_dollar_hash_brackets_matches_escaped_compile() {
    assert_same_literal_rendering("cost $5 #not-markup [brackets]");
}

#[test]
fn compile_text_unicode_emoji_bidi_complex_script_matches_escaped_compile() {
    assert_same_literal_rendering("Revenue 🚀 שלום नमस्ते");
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
