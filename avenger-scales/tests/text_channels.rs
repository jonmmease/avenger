//! Scales to text marks' channels: baselines and styles by name, weights as numbers.

use arrow::array::{ArrayRef, Float32Array, StringArray};
use avenger_common::types::{FontStyle, FontWeight, TextBaseline};
use avenger_scales::scales::{linear::LinearScale, ordinal::OrdinalScale};
use std::sync::Arc;

fn strings(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

#[test]
fn ordinal_scales_reach_every_baseline_and_style() {
    let domain = strings(&["a", "b", "c"]);
    let values = strings(&["a", "b", "c"]);
    let baselines = OrdinalScale::configured(domain.clone())
        .with_range(strings(&["line-top", "line-bottom", "alphabetic"]))
        .scale_to_text_baseline(&values)
        .unwrap();
    assert_eq!(
        baselines.as_vec(3, None),
        [
            TextBaseline::LineTop,
            TextBaseline::LineBottom,
            TextBaseline::Alphabetic
        ]
    );
    let styles = OrdinalScale::configured(domain)
        .with_range(strings(&["oblique", "italic", "normal"]))
        .scale_to_font_style(&values)
        .unwrap();
    assert_eq!(
        styles.as_vec(3, None),
        [FontStyle::Oblique, FontStyle::Italic, FontStyle::Normal]
    );
}

#[test]
fn weights_scale_as_numbers() {
    let ordinal = OrdinalScale::configured(strings(&["regular", "heavy"]))
        .with_range(Arc::new(Float32Array::from(vec![400.0, 700.0])))
        .scale_to_font_weight(&strings(&["heavy", "regular"]))
        .unwrap();
    assert_eq!(
        ordinal.as_vec(2, None),
        [FontWeight::BOLD, FontWeight::NORMAL]
    );

    let linear = LinearScale::configured((0.0, 1.0), (100.0, 900.0))
        .scale_to_font_weight(&(Arc::new(Float32Array::from(vec![0.5, 0.26])) as ArrayRef))
        .unwrap();
    assert_eq!(linear.as_vec(2, None), [FontWeight(500), FontWeight(308)]);
}
