//! Columns of spans: structs of a `start` and an `end`, which give a position and its partner.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float64Array, StructArray},
    datatypes::{DataType, Field, Float64Type},
    record_batch::RecordBatch,
};
use avenger_chart::{
    prelude::*,
    proto::{cartesian_layer, encoding, node, plot, scale},
    span,
};

/// A table of bins, `bin`, a span column of `start` and `end`, `edge`, a column of numbers, and
/// `count`.
fn bins() -> RecordBatch {
    let numbers = |values: &[f64]| Arc::new(Float64Array::from(values.to_vec())) as ArrayRef;
    let spans = StructArray::from(vec![
        (
            Arc::new(Field::new("start", DataType::Float64, false)),
            numbers(&[0.0, 5.0, 10.0]),
        ),
        (
            Arc::new(Field::new("end", DataType::Float64, false)),
            numbers(&[5.0, 10.0, 15.0]),
        ),
    ]);
    RecordBatch::try_from_iter([
        ("bin", Arc::new(spans) as ArrayRef),
        ("edge", numbers(&[5.0, 10.0, 15.0])),
        ("count", numbers(&[2.0, 5.0, 3.0])),
    ])
    .unwrap()
}

/// Fails unless `result` is invalid for a reason that contains `reason`.
#[track_caller]
fn invalid(result: Result<Chart, ChartError>, reason: &str) {
    match result {
        Err(ChartError::Invalid { reason: why, .. }) => {
            assert!(why.contains(reason), "`{why}` says `{reason}`")
        }
        other => panic!("expected an invalid chart, got {other:?}"),
    }
}

fn read(layer: CartesianLayerBuilder) -> Result<Chart, ChartError> {
    Chart::builder(Plot::cartesian([layer])).build()
}

#[test]
fn a_span_gives_its_channel_and_its_partner() {
    let rects = CartesianLayer::rect(bins(), RectMark::builder().x("bin").y("count").y2(0.0));
    let chart = read(rects).unwrap();
    let Some(node::Node::Plot(plot)) = &chart.root().node else {
        panic!("the root is a plot")
    };
    let Some(plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("the plot is Cartesian")
    };
    let Some(cartesian_layer::Mark::Rect(rect)) = &cartesian.layers[0].mark else {
        panic!("the layer draws rects")
    };
    // The reader gives `x2` the same column, and the scale takes the type of the fields.
    assert_eq!(
        rect.x2.as_ref().unwrap().kind,
        Some(encoding::Kind::ScaledField("bin".to_string()))
    );
    let x = rect.x.as_ref().unwrap().scale.as_ref().unwrap();
    assert!(matches!(x.r#type, Some(scale::Type::Linear(_))));
    // A chart that names the span on both channels reads as the same chart.
    let both = read(CartesianLayer::rect(
        bins(),
        RectMark::builder().x("bin").x2("bin").y("count").y2(0.0),
    ))
    .unwrap();
    assert_eq!(both, chart);
}

#[test]
fn the_first_channel_takes_the_starts_and_the_partner_the_ends() {
    let column = bins().column_by_name("bin").unwrap().clone();
    let starts = span::channel_values(&column, Channel::X);
    let ends = span::channel_values(&column, Channel::X2);
    assert_eq!(
        starts.as_primitive::<Float64Type>().values(),
        &[0.0, 5.0, 10.0]
    );
    assert_eq!(
        ends.as_primitive::<Float64Type>().values(),
        &[5.0, 10.0, 15.0]
    );
    // A column that is not a span is the values of every channel.
    let edges = bins().column_by_name("edge").unwrap().clone();
    assert_eq!(
        span::channel_values(&edges, Channel::X2).as_ref(),
        edges.as_ref()
    );
}

#[test]
fn a_span_and_another_partner_are_a_conflict() {
    invalid(
        read(CartesianLayer::rect(
            bins(),
            RectMark::builder().x("bin").x2("edge").y("count").y2(0.0),
        )),
        "the column of spans `bin` gives `x` and `x2`",
    );
}

#[test]
fn a_mark_without_the_partner_takes_no_span() {
    invalid(
        read(CartesianLayer::symbol(
            bins(),
            SymbolMark::builder().x("bin").y("count"),
        )),
        "a `symbol` mark takes no `x2`",
    );
    invalid(
        read(CartesianLayer::symbol(
            bins(),
            SymbolMark::builder().x("count").y("count").color("bin"),
        )),
        "`color` has no partner",
    );
}
