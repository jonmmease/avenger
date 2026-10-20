//! `format_array` labels Arrow arrays' values as `format` labels typed values: numbers of any
//! numeric type, dates, and naive or zoned timestamps, with other values as their text.
#![cfg(feature = "arrow")]

mod common;

use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Date32Array, Int32Array, StringArray, TimestampMillisecondArray,
};
use avenger_format::TickSpacing;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_typst_label::{LabelEngine, LabelValue};

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options())
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// The labels of `values` through `pattern`, as text.
fn labels(values: ArrayRef, pattern: &str) -> Vec<String> {
    engine()
        .format_array(&values, pattern, TickSpacing::Varying)
        .unwrap()
        .into_iter()
        .map(|value| match value {
            LabelValue::Str(text) | LabelValue::Markup(text) => text,
            value => panic!("{value:?} is not text"),
        })
        .collect()
}

#[test]
fn numbers_dates_and_timestamps_take_the_providers() {
    let numbers: ArrayRef = Arc::new(Int32Array::from(vec![1500, 25]));
    assert_eq!(labels(numbers, ",d"), ["1,500", "25"]);

    // 19 723 days after the epoch is 2024-01-01.
    let dates: ArrayRef = Arc::new(Date32Array::from(vec![19_723]));
    assert_eq!(labels(dates, "%Y-%m-%d"), ["2024-01-01"]);

    let millis = 1_704_110_400_000; // 2024-01-01T12:00:00Z
    let naive: ArrayRef = Arc::new(TimestampMillisecondArray::from(vec![millis]));
    assert_eq!(labels(naive, "%H:%M"), ["12:00"]);
    let zoned: ArrayRef =
        Arc::new(TimestampMillisecondArray::from(vec![millis]).with_timezone("UTC"));
    assert_eq!(labels(zoned, "%H:%M"), ["12:00"]);
}

/// Values that no provider formats are their text, and a null among them is empty text.
#[test]
fn other_values_are_their_text() {
    let strings: ArrayRef = Arc::new(StringArray::from(vec![Some("setosa"), None]));
    assert_eq!(labels(strings, ""), ["setosa", ""]);
    let booleans: ArrayRef = Arc::new(BooleanArray::from(vec![true]));
    assert_eq!(labels(booleans, ""), ["true"]);
}
