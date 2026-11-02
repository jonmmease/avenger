//! The rule that keeps a gridded range out of a chart.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array};
use avenger_chart::{proto, Chart, Clause, Condition, PixelGrid, Selection};
use avenger_scales::scales::linear::LinearScale;

fn values(values: &[Option<f64>]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

/// The x of a plot 200 pixels wide over the domain 0 to 100, with cells of 10 pixels.
fn x_grid() -> PixelGrid {
    PixelGrid::new(LinearScale::configured((0.0, 100.0), (0.0, 200.0)), 10.0).unwrap()
}

/// A chart cannot store a gridded range as an initial clause, because without its grid the
/// range would select other rows.
#[test]
fn a_gridded_range_is_no_initial_clause() {
    let table = arrow::array::RecordBatch::try_from_iter([
        ("x", values(&[Some(1.0), Some(2.0)])),
        ("y", values(&[Some(1.0), Some(2.0)])),
    ])
    .unwrap();
    let brush = Selection::new("brush", proto::SelectionResolution::Global).initial(
        Clause::new("points").conditions([Condition::Range {
            column: "x".into(),
            channel: proto::Channel::X.label(),
            ends: values(&[Some(10.0), Some(50.0)]),
            exclude_end: false,
            cells: Some(x_grid().span(20.0, 100.0).unwrap()),
        }]),
    );
    let error = Chart::unit(table, proto::SymbolMark::default())
        .id("points")
        .x("x")
        .y("y")
        .selection(brush)
        .build()
        .unwrap_err();
    assert!(error.to_string().contains("selection `brush`"), "{error}");
}
