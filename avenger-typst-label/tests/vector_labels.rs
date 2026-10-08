//! A label's vector drawing items, in its box.

mod common;

use avenger_typst_label::{Label, LabelEngine, LabelOptions, LabelSource, SvgItem};

fn label(source: LabelSource<'_>) -> Label<'_> {
    let mut options = LabelOptions::default();
    options.text.font_family = "Lato".into();
    options.text.font_size = 14.0;
    Label { source, options }
}

fn runs(items: &[SvgItem]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            SvgItem::Text(run) => Some(run.text.as_str()),
            _ => None,
        })
        .collect()
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
fn svg_draws_invalid_markup_as_its_source() {
    let engine = LabelEngine::new(common::engine_options());
    let (_, svg) = engine.svg(&label(LabelSource::Markup("a $b"))).unwrap();
    assert_eq!(runs(&svg.items), "a $b");
}
