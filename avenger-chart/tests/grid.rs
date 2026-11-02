//! Pixel grids: the cells of values through a plot's scale, the cells of a box, and the rule that
//! keeps a gridded range out of a chart.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, Int64Array};
use avenger_chart::{proto, Chart, Contribution, PixelGrid, Selection, TupleTest};
use avenger_scales::scales::linear::LinearScale;

fn values(values: &[Option<f64>]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

/// The x of a plot 200 pixels wide over the domain 0 to 100, with cells of 10 pixels.
fn x_grid() -> PixelGrid {
    PixelGrid::new(LinearScale::configured((0.0, 100.0), (0.0, 200.0)), 10.0).unwrap()
}

/// A value's cell is the floor of its pixel less the origin, over the cell width.
#[test]
fn values_take_the_cells_of_their_pixels() {
    let cells = x_grid()
        .cells(&values(&[Some(0.0), Some(4.9), Some(5.0), Some(99.9)]))
        .unwrap();
    assert_eq!(cells, Int64Array::from(vec![0, 0, 1, 19]));
}

/// A decreasing range, as a plot's y has, counts its cells from the smaller end of the range.
#[test]
fn a_decreasing_range_counts_from_its_smaller_end() {
    let grid = PixelGrid::new(LinearScale::configured((0.0, 10.0), (100.0, 0.0)), 10.0).unwrap();
    assert_eq!(grid.origin, 0.0);
    let cells = grid
        .cells(&values(&[Some(10.0), Some(8.0), Some(0.5)]))
        .unwrap();
    assert_eq!(cells, Int64Array::from(vec![0, 2, 9]));
}

/// A null value and a value whose pixel is not finite have no cell.
#[test]
fn values_without_finite_pixels_have_no_cell() {
    let cells = x_grid()
        .cells(&values(&[None, Some(f64::NAN), Some(f64::INFINITY)]))
        .unwrap();
    assert_eq!(cells, Int64Array::from(vec![None, None, None]));
}

/// A scale that clamps puts a value outside its domain in an end cell, and one that does not
/// puts it outside the plot.
#[test]
fn clamping_follows_the_scale() {
    let outside = values(&[Some(-10.0), Some(110.0)]);
    assert_eq!(
        x_grid().cells(&outside).unwrap(),
        Int64Array::from(vec![-2, 22])
    );
    let clamped = LinearScale::configured((0.0, 100.0), (0.0, 200.0)).with_option("clamp", true);
    let grid = PixelGrid::new(clamped, 10.0).unwrap();
    assert_eq!(grid.cells(&outside).unwrap(), Int64Array::from(vec![0, 20]));
}

/// A box holds the cell of its near edge through the last cell that it reaches into, so a far
/// edge on a cell boundary adds no cell, and a box covers the values of its cells.
#[test]
fn a_box_holds_the_cells_that_it_reaches_into() {
    let snapped = x_grid().span(20.0, 100.0).unwrap();
    assert_eq!([snapped.first, snapped.last], [2, 9]);
    let smooth = x_grid().span(20.5, 100.25).unwrap();
    assert_eq!([smooth.first, smooth.last], [2, 10]);
    let narrow = x_grid().span(23.0, 23.0).unwrap();
    assert_eq!([narrow.first, narrow.last], [2, 2]);

    // Cells 2 to 9 are the values 10 to 50, without 50 itself, whose pixel starts cell 10.
    let selected = snapped
        .contains(&values(&[Some(9.99), Some(10.0), Some(49.99), Some(50.0)]))
        .unwrap();
    assert_eq!(selected, vec![false, true, true, false]);
}

/// Grids are the same when their scales, origins, and cells are.
#[test]
fn grids_compare_by_scale_origin_and_cell() {
    assert!(x_grid().same(&x_grid()));
    let wider = PixelGrid::new(LinearScale::configured((0.0, 100.0), (0.0, 400.0)), 10.0).unwrap();
    assert!(!x_grid().same(&wider));
    let finer = PixelGrid::new(LinearScale::configured((0.0, 100.0), (0.0, 200.0)), 5.0).unwrap();
    assert!(!x_grid().same(&finer));
    assert!(PixelGrid::new(LinearScale::configured((0.0, 1.0), (0.0, 1.0)), 0.0).is_err());
}

/// A chart cannot store a gridded range as an initial contribution, because without its grid the
/// range would select other rows.
#[test]
fn a_gridded_range_is_no_initial_contribution() {
    let table = arrow::array::RecordBatch::try_from_iter([
        ("x", values(&[Some(1.0), Some(2.0)])),
        ("y", values(&[Some(1.0), Some(2.0)])),
    ])
    .unwrap();
    let brush = Selection::new("brush", proto::SelectionResolution::Global).initial(
        Contribution::new("points").tuple([TupleTest::Range {
            column: "x".into(),
            channel: proto::Channel::X,
            ends: values(&[Some(10.0), Some(50.0)]),
            exclude_end: false,
            grid: Some(x_grid().span(20.0, 100.0).unwrap()),
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
