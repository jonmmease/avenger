//! Scales to pattern fills, which read each pattern from its text.

use arrow::array::{ArrayRef, DictionaryArray, StringArray};
use arrow::datatypes::Int32Type;
use avenger_common::pattern::{PatternFill, PatternLayer, StripeLayer};
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scales::{coerce::Coercer, ordinal::OrdinalScale};
use std::sync::Arc;

fn strings(values: &[Option<&str>]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn stripes(angle: f32) -> PatternFill {
    PatternFill {
        layers: vec![PatternLayer::from(StripeLayer::new(angle, 8.0, 2.0))],
        ..Default::default()
    }
}

#[test]
fn ordinal_scales_map_categories_to_patterns() {
    let patterns = [stripes(45.0), stripes(-45.0), PatternFill::default()];
    let scale = OrdinalScale::configured(strings(&[Some("a"), Some("b"), Some("c")]))
        .with_range_patterns(&patterns);
    let scaled = scale
        .scale_to_pattern(&strings(&[
            Some("b"),
            Some("a"),
            Some("c"),
            Some("d"),
            None,
        ]))
        .unwrap();
    assert_eq!(
        scaled.as_vec(5, None),
        [Some(stripes(-45.0)), Some(stripes(45.0)), None, None, None]
    );
}

#[test]
fn patterns_read_their_text_and_nulls() {
    let patterns = Coercer::default()
        .to_pattern(&strings(&[Some("stripes 45 8 2"), Some("none"), None]))
        .unwrap();
    assert_eq!(patterns.as_vec(3, None), [Some(stripes(45.0)), None, None]);

    let keys = DictionaryArray::<Int32Type>::from_iter([
        Some("stripes 45 8 2"),
        None,
        Some("stripes 45 8 2"),
    ]);
    let patterns = Coercer::default()
        .to_pattern(&(Arc::new(keys) as ArrayRef))
        .unwrap();
    assert_eq!(
        patterns.as_vec(3, None),
        [Some(stripes(45.0)), None, Some(stripes(45.0))]
    );
}

#[test]
fn invalid_patterns_quote_their_text() {
    let too_many = ["stripes 0 8 1"; PatternFill::MAX_LAYERS + 1].join(" + ");
    for text in [
        // Text that does not follow the grammar.
        "stripes 45",
        // A stripe without room between its lines.
        "stripes 0 0 1",
        too_many.as_str(),
    ] {
        match Coercer::default().to_pattern(&strings(&[Some(text)])) {
            Err(AvengerScaleError::InvalidPattern { pattern, .. }) => assert_eq!(pattern, text),
            other => panic!("expected an invalid pattern for {text}, got {other:?}"),
        }
    }
}
