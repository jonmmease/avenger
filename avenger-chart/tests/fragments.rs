//! Fragments: a message of the format with the values that its references refer to, the serialized
//! form of its builder.

mod support;

use std::sync::Arc;

use arrow::array::{ArrayRef, Date32Array};
use avenger_chart::{prelude::*, proto::Fragment, FORMAT_VERSION};
use prost::Message;
use support::points;

/// Fails unless `result` is invalid at `path`, for a reason that contains `reason`.
#[track_caller]
fn invalid<T: std::fmt::Debug>(result: Result<T, ChartError>, path: &str, reason: &str) {
    match result {
        Err(ChartError::Invalid {
            path: at,
            reason: why,
        }) => {
            assert_eq!(at, path, "the error names the path ({why})");
            assert!(why.contains(reason), "`{why}` says `{reason}`");
        }
        other => panic!("expected an error at {path}, got {other:?}"),
    }
}

fn dates() -> ArrayRef {
    Arc::new(Date32Array::from(vec![19000, 19400]))
}

/// A fit with values of two types: a list of dates and a number.
fn fit() -> IntervalFitBuilder {
    IntervalFit::builder()
        .empty(Value::list(dates()))
        .pin(DomainPin::new(0.0, PinPosition::Low))
}

/// A chart of symbols whose `x` scale fits its domain with `fit`.
fn chart(fit: IntervalFitBuilder) -> Chart {
    let scale = Scale::linear(LinearScale::builder()).domain(DataDomain::builder().fit(fit));
    let mark = SymbolMark::builder()
        .x(Encoding::scaled_field("a").scale(scale))
        .y("b");
    let layer = CartesianLayer::symbol(points(), mark);
    Chart::builder(Plot::cartesian([layer])).build().unwrap()
}

#[test]
fn a_builder_round_trips_through_its_fragment() {
    let fragment = fit().to_fragment().unwrap();
    assert_eq!(fragment.version, FORMAT_VERSION);
    assert_eq!(fragment.r#type, "avenger.chart.v1.IntervalFit");
    assert!(fragment.values.is_some(), "the fragment holds the values");
    let decoded = IntervalFitBuilder::from_fragment(&fragment).unwrap();
    assert_eq!(chart(decoded), chart(fit()));
}

#[test]
fn a_message_without_values_has_no_values() {
    let fragment = IntervalFit::builder().zero(true).to_fragment().unwrap();
    assert!(fragment.values.is_none());
    let decoded = IntervalFitBuilder::from_fragment(&fragment).unwrap();
    assert_eq!(decoded.build().unwrap().zero, Some(true));
}

#[test]
fn the_bytes_are_canonical() {
    let first = IntervalFit::builder()
        .empty(Value::list(dates()))
        .pin(DomainPin::new(0.0, PinPosition::Low))
        .to_fragment()
        .unwrap();
    let second = IntervalFit::builder()
        .pin(DomainPin::new(0.0, PinPosition::Low))
        .empty(Value::list(dates()))
        .to_fragment()
        .unwrap();
    assert_eq!(first.encode_to_vec(), second.encode_to_vec());
}

#[test]
fn a_fragment_of_another_type_does_not_decode() {
    let fragment = fit().to_fragment().unwrap();
    invalid(
        ScaleBuilder::from_fragment(&fragment),
        "type",
        "holds a `avenger.chart.v1.IntervalFit`, not a `avenger.chart.v1.Scale`",
    );
}

#[test]
fn a_fragment_of_another_version_does_not_decode() {
    let fragment = Fragment {
        version: FORMAT_VERSION + 1,
        ..fit().to_fragment().unwrap()
    };
    invalid(
        IntervalFitBuilder::from_fragment(&fragment),
        "version",
        "this reader reads version",
    );
}

#[test]
fn a_reference_outside_the_values_does_not_decode() {
    let fragment = Fragment {
        values: None,
        ..fit().to_fragment().unwrap()
    };
    invalid(
        IntervalFitBuilder::from_fragment(&fragment),
        "values",
        "refers to value",
    );
}

#[test]
fn bytes_that_are_not_the_message_do_not_decode() {
    let fragment = Fragment {
        message: vec![0xff, 0xff, 0xff].into(),
        ..fit().to_fragment().unwrap()
    };
    assert!(IntervalFitBuilder::from_fragment(&fragment).is_err());
}

#[test]
fn a_fragment_holds_no_tables_or_charts() {
    let layer = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"));
    invalid(layer.to_fragment(), "table", "holds a table or a chart");
    let node = NodeBuilder::from(chart(fit()));
    invalid(node.to_fragment(), "", "holds a table or a chart");
}

#[test]
fn a_fragment_reports_the_first_error_of_the_setters() {
    let fit = IntervalFit::builder().empty(Value::scalar(dates()));
    invalid(fit.to_fragment(), "empty", "an array of one element");
}
