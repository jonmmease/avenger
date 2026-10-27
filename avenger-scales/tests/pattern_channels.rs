//! Scales to pattern fills, which read the JSON of each pattern's serde form.

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
    let patterns = [stripes(45.0), stripes(-45.0)];
    let scale = OrdinalScale::configured(strings(&[Some("a"), Some("b")]))
        .with_range_patterns(&patterns)
        .unwrap();
    let scaled = scale
        .scale_to_pattern(&strings(&[Some("b"), Some("a"), Some("c"), None]))
        .unwrap();
    assert_eq!(
        scaled.as_vec(4, None),
        [Some(stripes(-45.0)), Some(stripes(45.0)), None, None]
    );
}

#[test]
fn patterns_read_hand_written_json_and_nulls() {
    let json = r#"{"layers": [{"type": "stripe", "angle": 45, "spacing": 8, "stroke-width": 2}]}"#;
    let patterns = Coercer::default()
        .to_pattern(&strings(&[Some(json), None]))
        .unwrap();
    assert_eq!(patterns.as_vec(2, None), [Some(stripes(45.0)), None]);

    let keys = DictionaryArray::<Int32Type>::from_iter([Some(json), None, Some(json)]);
    let patterns = Coercer::default()
        .to_pattern(&(Arc::new(keys) as ArrayRef))
        .unwrap();
    assert_eq!(
        patterns.as_vec(3, None),
        [Some(stripes(45.0)), None, Some(stripes(45.0))]
    );
}

#[test]
fn invalid_patterns_name_their_json() {
    let coercer = Coercer::default();
    for json in [
        // Not JSON.
        "stripes",
        // A stripe without room between its lines.
        r#"{"layers": [{"type": "stripe", "angle": 0, "spacing": 0, "stroke-width": 1}]}"#,
    ] {
        match coercer.to_pattern(&strings(&[Some(json)])) {
            Err(AvengerScaleError::InvalidPattern { pattern, .. }) => assert_eq!(pattern, json),
            other => panic!("expected an invalid pattern for {json}, got {other:?}"),
        }
    }

    let too_many = PatternFill {
        layers: vec![stripes(0.0).layers[0].clone(); PatternFill::MAX_LAYERS + 1],
        ..Default::default()
    };
    let json = serde_json::to_string(&too_many).unwrap();
    assert!(matches!(
        coercer.to_pattern(&strings(&[Some(&json)])),
        Err(AvengerScaleError::InvalidPattern { .. })
    ));
}
