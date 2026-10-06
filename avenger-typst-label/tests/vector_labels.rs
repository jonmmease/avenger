//! A label's vector drawing items, in its box.

mod common;

use avenger_typst_label::{
    Label, LabelEngine, LabelOptions, LabelSource, PdfItem, SvgItem,
};

fn label(source: LabelSource<'_>) -> Label<'_> {
    let mut options = LabelOptions::default();
    options.text.font_family = "Lato".into();
    options.text.font_size = 14.0;
    Label { source, options }
}

#[test]
fn svg_items_sit_in_the_labels_box() {
    let engine = LabelEngine::new(common::engine_options());
    let (bounds, svg) = engine.svg(&label(LabelSource::Text("Ag"))).unwrap();
    assert_eq!((svg.size.x, svg.size.y), (bounds.width, bounds.height));
    let [SvgItem::Text(run)] = svg.items.as_slice() else {
        panic!("{:?}", svg.items);
    };
    assert!((run.baseline - bounds.ascent).abs() < 1e-4, "{run:?} {bounds:?}");
    assert_eq!(engine.bounds(&label(LabelSource::Text("Ag"))).unwrap(), bounds);
}

#[test]
fn pdf_glyph_runs_sit_on_the_boxs_baseline() {
    let engine = LabelEngine::new(common::engine_options());
    let (bounds, pdf) = engine.pdf(&label(LabelSource::Text("Hello"))).unwrap();
    assert_eq!((pdf.size.x, pdf.size.y), (bounds.width, bounds.height));
    let [PdfItem::Text(run)] = pdf.items.as_slice() else {
        panic!("{:?}", pdf.items);
    };
    assert_eq!(run.text, "Hello");
    // The baseline lies the box's ascent below its top.
    let y = run.transform.apply(run.glyphs[0].position).y;
    assert!((y - bounds.ascent).abs() < 1e-3, "{y}: {bounds:?}");
}

#[test]
fn pdf_math_draws_glyph_runs_and_its_fraction_line_as_a_path() {
    let engine = LabelEngine::new(common::engine_options());
    let (_, pdf) = engine.pdf(&label(LabelSource::Markup("$a / b$"))).unwrap();
    let runs = pdf.items.iter().filter(|item| matches!(item, PdfItem::Text(_)));
    assert!(runs.count() >= 2, "{:?}", pdf.items);
    assert!(pdf.items.iter().any(|item| matches!(item, PdfItem::Path(_))));
}

#[test]
fn invalid_markup_draws_its_source_as_text() {
    let engine = LabelEngine::new(common::engine_options());
    let invalid = label(LabelSource::Markup("a $b"));
    let (_, svg) = engine.svg(&invalid).unwrap();
    let svg_text: String = svg
        .items
        .iter()
        .filter_map(|item| match item {
            SvgItem::Text(run) => Some(run.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(svg_text, "a $b");
    let (_, pdf) = engine.pdf(&invalid).unwrap();
    assert_eq!(pdf.semantic_text, "a $b");
}
