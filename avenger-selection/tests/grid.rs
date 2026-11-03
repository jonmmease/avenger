//! Pixel grids: the cells of values through a plot's scale, the cells of a box, and the cells
//! that a lasso covers.

use std::sync::Arc;

use arrow::array::{ArrayRef, BooleanArray, Float64Array, Int64Array};
use arrow::datatypes::DataType;
use avenger_scales::scales::linear::LinearScale;
use avenger_selection::{Condition, PixelGrid, PolygonCells};

mod common;

use common::Noise;

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
/// edge on a cell boundary adds no cell.
#[test]
fn a_box_holds_the_cells_that_it_reaches_into() {
    let snapped = x_grid().span(20.0, 100.0).unwrap();
    assert_eq!([snapped.first, snapped.last], [2, 9]);
    let smooth = x_grid().span(20.5, 100.25).unwrap();
    assert_eq!([smooth.first, smooth.last], [2, 10]);
    let narrow = x_grid().span(23.0, 23.0).unwrap();
    assert_eq!([narrow.first, narrow.last], [2, 2]);
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

/// A grid of 1-pixel cells over a 10-pixel range of the domain 0 to 10, so a value is its pixel.
fn unit_grid() -> PixelGrid {
    PixelGrid::new(LinearScale::configured((0.0, 10.0), (0.0, 10.0)), 1.0).unwrap()
}

/// Whether the point of each of `x` and `y` lies in one of `cells`, through the polygon condition
/// of the cells.
fn lies_in(cells: &PolygonCells, x: &ArrayRef, y: &ArrayRef) -> BooleanArray {
    lasso(cells).matches(&[x.clone(), y.clone()]).unwrap()
}

/// The polygon condition of `cells`.
fn lasso(cells: &PolygonCells) -> Condition {
    Condition::Polygon {
        x_column: "x".into(),
        y_column: "y".into(),
        polygons: Vec::new(),
        cells: Some(cells.clone()),
    }
}

/// Whether each pair of a cell of `columns` and a cell of `rows` is one of `cells`, through the
/// keys of the polygon condition of the cells.
fn covers(cells: &PolygonCells, columns: &Int64Array, rows: &Int64Array) -> BooleanArray {
    let prepared = lasso(cells).prepare(&[DataType::Int64, DataType::Int64]);
    let keys: [ArrayRef; 2] = [Arc::new(columns.clone()), Arc::new(rows.clone())];
    prepared.unwrap().matches_keys(&keys).unwrap()
}

/// A lasso covers the cells whose centers are inside it, by the even-odd rule, row by row.
#[test]
fn a_lasso_covers_the_cells_whose_centers_are_inside() {
    // A square from 1.2 to 3.8 holds the centers 1.5, 2.5, and 3.5 of each dimension.
    let square = vec![[1.2, 1.2], [3.8, 1.2], [3.8, 3.8], [1.2, 3.8]];
    let cells = PolygonCells::cover([unit_grid(), unit_grid()], &[square]);
    assert_eq!(cells.first_row, 1);
    assert_eq!(cells.rows, vec![vec![[1, 3]]; 3]);

    // A U from 0 to 6 whose notch, from 2 to 4, reaches down to 3 leaves two runs in its arms.
    let notched = vec![
        [0.0, 0.0],
        [6.0, 0.0],
        [6.0, 6.0],
        [4.0, 6.0],
        [4.0, 3.0],
        [2.0, 3.0],
        [2.0, 6.0],
        [0.0, 6.0],
    ];
    let cells = PolygonCells::cover([unit_grid(), unit_grid()], &[notched]);
    assert_eq!(cells.first_row, 0);
    assert_eq!(cells.rows[..3], vec![vec![[0, 5]]; 3]);
    assert_eq!(cells.rows[3..], vec![vec![[0, 1], [4, 5]]; 3]);
}

/// The cells of several lassos are their union, with overlapping and touching runs merged.
#[test]
fn the_cells_of_several_lassos_merge() {
    let square = |low: f64, high: f64| vec![[low, 0.0], [high, 0.0], [high, 1.0], [low, 1.0]];
    let cells = PolygonCells::cover(
        [unit_grid(), unit_grid()],
        &[
            square(0.0, 3.0),
            square(2.0, 5.0),
            square(5.0, 6.0),
            square(8.0, 9.0),
        ],
    );
    assert_eq!(cells.rows, vec![vec![[0, 5], [8, 8]]]);
}

/// A point matches the cells when its cell in each grid is one of them, and a point without a
/// cell matches none.
#[test]
fn points_match_the_cells_that_they_lie_in() {
    let square = vec![[1.0, 1.0], [4.0, 1.0], [4.0, 3.0], [1.0, 3.0]];
    let cells = PolygonCells::cover([unit_grid(), unit_grid()], &[square]);
    let x = values(&[
        Some(1.0),
        Some(3.99),
        Some(4.0),
        Some(2.0),
        None,
        Some(0.5),
        Some(2.0),
    ]);
    let y = values(&[
        Some(1.0),
        Some(2.99),
        Some(1.5),
        Some(3.0),
        Some(2.0),
        Some(2.0),
        Some(f64::NAN),
    ]);
    assert_eq!(
        lies_in(&cells, &x, &y),
        BooleanArray::from(vec![true, true, false, false, false, false, false])
    );
    let empty = PolygonCells::cover([unit_grid(), unit_grid()], &[vec![[1.0, 1.0], [1.2, 1.0]]]);
    assert!(empty.rows.is_empty());
    assert_eq!(lies_in(&empty, &x, &y), BooleanArray::from(vec![false; 7]));
}

/// The cells cover the pairs of cells that they hold, as a table of rows grouped by their cells
/// holds them, and no pair with a null cell.
#[test]
fn the_cells_cover_their_pairs_of_cells() {
    let square = vec![[1.0, 1.0], [4.0, 1.0], [4.0, 3.0], [1.0, 3.0]];
    let cells = PolygonCells::cover([unit_grid(), unit_grid()], &[square]);
    let columns = Int64Array::from(vec![
        Some(1),
        Some(3),
        Some(4),
        Some(2),
        None,
        Some(0),
        Some(2),
    ]);
    let rows = Int64Array::from(vec![
        Some(1),
        Some(2),
        Some(1),
        Some(3),
        Some(2),
        Some(2),
        None,
    ]);
    assert_eq!(
        covers(&cells, &columns, &rows),
        BooleanArray::from(vec![true, true, false, false, false, false, false])
    );
}

/// A jagged lasso over a plot of 300 by 200 pixels matches exactly the points whose cells have
/// their centers inside it, by the even-odd rule over every edge.
#[test]
fn a_lasso_matches_the_points_of_its_covered_cells() {
    let x_grid = PixelGrid::new(LinearScale::configured((0.0, 1.0), (0.0, 300.0)), 2.0).unwrap();
    let y_grid = PixelGrid::new(LinearScale::configured((0.0, 1.0), (200.0, 0.0)), 2.0).unwrap();
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let lasso: Vec<[f64; 2]> = (0..300)
        .map(|index| {
            let angle = index as f64 / 300.0 * std::f64::consts::TAU;
            let radius = 30.0 + 60.0 * noise.unit();
            [150.0 + radius * angle.cos(), 100.0 + radius * angle.sin()]
        })
        .collect();
    let cells = PolygonCells::cover(
        [x_grid.clone(), y_grid.clone()],
        std::slice::from_ref(&lasso),
    );
    let x: Vec<f64> = (0..20_000).map(|_| noise.unit()).collect();
    let y: Vec<f64> = (0..20_000).map(|_| noise.unit()).collect();
    let (x, y) = (
        values(&x.iter().map(|v| Some(*v)).collect::<Vec<_>>()),
        values(&y.iter().map(|v| Some(*v)).collect::<Vec<_>>()),
    );
    let matched = lies_in(&cells, &x, &y);
    let (columns, rows) = (x_grid.cells(&x).unwrap(), y_grid.cells(&y).unwrap());
    let covered = covers(&cells, &columns, &rows);
    let center = |grid: &PixelGrid, cell: i64| grid.origin + (cell as f64 + 0.5) * grid.cell;
    let mut inside = 0;
    for index in 0..x.len() {
        let (px, py) = (
            center(&x_grid, columns.value(index)),
            center(&y_grid, rows.value(index)),
        );
        let mut expected = false;
        for (i, &[xi, yi]) in lasso.iter().enumerate() {
            let [xj, yj] = lasso[(i + lasso.len() - 1) % lasso.len()];
            if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
                expected = !expected;
            }
        }
        assert_eq!(matched.value(index), expected, "the point {index}");
        assert_eq!(
            covered.value(index),
            expected,
            "the cells of the point {index}"
        );
        inside += expected as usize;
    }
    assert!(inside > 1_000 && inside < 19_000, "{inside} points inside");
}

/// A range with cells matches the keys of its cells, a null key matches none, and a range
/// without cells has no keys.
#[test]
fn a_range_matches_the_keys_of_its_cells() {
    let range = |cells| Condition::Range {
        column: "x".into(),
        channel: None,
        ends: values(&[Some(10.0), Some(50.0)]),
        exclude_end: false,
        cells,
    };
    let gridded = range(Some(x_grid().span(20.0, 100.0).unwrap()));
    let keys: ArrayRef = Arc::new(Int64Array::from(vec![
        Some(1),
        Some(2),
        Some(9),
        Some(10),
        None,
    ]));
    let prepared = gridded.prepare(&[DataType::Int64]).unwrap();
    assert_eq!(
        prepared.matches_keys(std::slice::from_ref(&keys)).unwrap(),
        BooleanArray::from(vec![false, true, true, false, false])
    );
    let plain = range(None).prepare(&[DataType::Int64]).unwrap();
    assert!(plain.matches_keys(&[keys]).is_err());
}
