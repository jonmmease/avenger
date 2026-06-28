use avenger_typst_label::{
    CompiledLabel, EngineOptions, LabelEngine, LabelFrameItem, LabelOptions, LabelParamValue,
    PdfDrawItem, PdfOptions, SvgOptions, TextItemKind, escape_text, pdf_items, svg_items,
};

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
    let engine = LabelEngine::new(EngineOptions::default()).unwrap();
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
fn final_public_api_literal_fast_path_matches_escaped_markup() {
    let engine = LabelEngine::new(EngineOptions::default()).unwrap();
    let options = LabelOptions::default();
    let text = "cost $5 #literal [brackets] 🚀 שלום नमस्ते";

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
