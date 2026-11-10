//! The path order of lines and areas: each series connects its rows in ascending order of `order`,
//! or without it, along the key, `x` when vertical and `y` when horizontal, with equal values in
//! row order. The first row of each series orders the series. A row without a position breaks the
//! path.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::prelude::*;
use avenger_chart_eval::EvaluatedChart;
use avenger_scenegraph::marks::mark::SceneMark;

use crate::support::*;

/// The rows of `key`, `value`, and `series`.
fn table(key: &[f64], value: &[f64], series: &[&str]) -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "key",
            Arc::new(Float64Array::from(key.to_vec())) as ArrayRef,
        ),
        ("value", Arc::new(Float64Array::from(value.to_vec()))),
        ("series", Arc::new(StringArray::from(series.to_vec()))),
    ])
    .unwrap()
}

/// Five rows of one series out of the order of their keys, and the same rows in it.
fn shuffled() -> (RecordBatch, RecordBatch) {
    let series = ["a"; 5];
    (
        table(
            &[3.0, 1.0, 4.0, 0.0, 2.0],
            &[6.0, 2.0, 8.0, 1.0, 4.0],
            &series,
        ),
        table(
            &[0.0, 1.0, 2.0, 3.0, 4.0],
            &[1.0, 2.0, 4.0, 6.0, 8.0],
            &series,
        ),
    )
}

/// The marks of the first plot of `chart`, as text.
fn scene(chart: &Chart) -> String {
    format!("{:?}", plot_group(&evaluate(chart, [320.0, 240.0])).marks)
}

/// The x and the y positions of each line and area of the first plot.
fn paths(evaluated: &EvaluatedChart) -> Vec<(Vec<f32>, Vec<f32>)> {
    (plot_group(evaluated).marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Line(line) => Some((
                line.x.as_vec(line.len as usize, None),
                line.y.as_vec(line.len as usize, None),
            )),
            SceneMark::Area(area) => Some((
                area.x.as_vec(area.len as usize, None),
                area.y.as_vec(area.len as usize, None),
            )),
            _ => None,
        })
        .collect()
}

fn ascending(values: &[f32]) -> bool {
    values.windows(2).all(|pair| pair[0] <= pair[1])
}

fn line(rows: RecordBatch, mark: LineMarkBuilder) -> Chart {
    Chart::builder(Plot::cartesian([CartesianLayer::line(rows, mark)]))
        .build()
        .unwrap()
}

fn area(rows: RecordBatch, mark: AreaMarkBuilder) -> Chart {
    Chart::builder(Plot::cartesian([CartesianLayer::area(rows, mark)]))
        .build()
        .unwrap()
}

#[test]
fn a_line_and_an_area_connect_their_rows_along_x() {
    let (rows, sorted) = shuffled();
    let mark = || LineMark::builder().x("key").y("value");
    assert_eq!(
        scene(&line(rows.clone(), mark())),
        scene(&line(sorted.clone(), mark()))
    );
    let mark = || AreaMark::builder().x("key").y("value");
    assert_eq!(
        scene(&area(rows.clone(), mark())),
        scene(&area(sorted, mark()))
    );
    let evaluated = evaluate(&area(rows, mark()), [320.0, 240.0]);
    let (x, _) = &paths(&evaluated)[0];
    assert!(ascending(x), "{x:?}");
}

#[test]
fn a_horizontal_line_and_area_connect_their_rows_along_y() {
    let (rows, sorted) = shuffled();
    let mark = || {
        LineMark::builder()
            .orient(Orient::Horizontal)
            .x("value")
            .y("key")
    };
    assert_eq!(
        scene(&line(rows.clone(), mark())),
        scene(&line(sorted.clone(), mark()))
    );
    let mark = || {
        AreaMark::builder()
            .orient(Orient::Horizontal)
            .x("value")
            .y("key")
    };
    assert_eq!(
        scene(&area(rows.clone(), mark())),
        scene(&area(sorted, mark()))
    );
    let evaluated = evaluate(&area(rows, mark()), [320.0, 240.0]);
    let (_, y) = &paths(&evaluated)[0];
    let mut descending = y.clone();
    descending.reverse();
    assert!(ascending(y) || ascending(&descending), "{y:?}");
}

/// Two quantitative positions are vertical, so a connected scatter plot writes `order`, and its
/// line connects the rows in the order of `order`, with `NULL` last.
#[test]
fn order_connects_the_rows_in_its_order() {
    let rows = RecordBatch::try_from_iter([
        (
            "x",
            Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0])) as ArrayRef,
        ),
        ("y", Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0]))),
        (
            "t",
            Arc::new(Float64Array::from(vec![
                Some(2.0),
                None,
                Some(0.0),
                Some(1.0),
            ])),
        ),
    ])
    .unwrap();
    let evaluated = evaluate(
        &line(rows, LineMark::builder().x("x").y("y").order("t")),
        [320.0, 240.0],
    );
    let (x, _) = &paths(&evaluated)[0];
    let sorted = {
        let mut sorted = x.clone();
        sorted.sort_by(f32::total_cmp);
        sorted
    };
    // The rows of `t` 0, 1, 2, and NULL: x 2, 3, 0, and 1.
    assert_eq!(*x, vec![sorted[2], sorted[3], sorted[0], sorted[1]]);
}

/// The first row of each series orders the series, and rows of one key keep their row order.
#[test]
fn the_first_rows_order_the_series_and_ties_keep_the_row_order() {
    let rows = table(
        &[2.0, 1.0, 0.0, 1.0, 1.0],
        &[5.0, 9.0, 1.0, 3.0, 4.0],
        &["b", "a", "b", "b", "a"],
    );
    let evaluated = evaluate(
        &line(
            rows,
            LineMark::builder().x("key").y("value").detail("series"),
        ),
        [320.0, 240.0],
    );
    let paths = paths(&evaluated);
    assert_eq!(paths.len(), 2);
    // Series b first: its rows at keys 0, 1, and 2.
    assert_eq!(paths[0].0.len(), 3);
    assert!(ascending(&paths[0].0));
    // Series a: two rows at key 1, values 9 then 4, in row order.
    let (x, y) = &paths[1];
    assert_eq!(x[0], x[1]);
    assert!(y[0] < y[1], "9 draws above 4: {y:?}");
}

/// An area from a baseline of rows out of order, as a stack draws, draws the region of its sorted
/// rows.
#[test]
fn an_area_from_a_baseline_draws_its_sorted_rows() {
    let rows = |key: [f64; 3], value: [f64; 3], base: [f64; 3]| {
        RecordBatch::try_from_iter([
            (
                "key",
                Arc::new(Float64Array::from(key.to_vec())) as ArrayRef,
            ),
            ("value", Arc::new(Float64Array::from(value.to_vec()))),
            ("base", Arc::new(Float64Array::from(base.to_vec()))),
        ])
        .unwrap()
    };
    let mark = || AreaMark::builder().x("key").y("value").y2("base");
    assert_eq!(
        scene(&area(
            rows([1.0, 0.0, 2.0], [3.0, 1.0, 2.0], [0.5, 0.2, 0.1]),
            mark()
        )),
        scene(&area(
            rows([0.0, 1.0, 2.0], [1.0, 3.0, 2.0], [0.2, 0.5, 0.1]),
            mark()
        ))
    );
}

#[test]
fn a_polar_area_connects_its_rows_along_theta() {
    let (rows, sorted) = shuffled();
    let chart = |rows: RecordBatch| {
        let areas = PolarLayer::area(
            rows.clone(),
            PolarAreaMark::builder()
                .theta("key")
                .radius("value")
                .fill_opacity(0.25),
        );
        let lines = PolarLayer::line(rows, PolarLineMark::builder().theta("key").radius("value"));
        Chart::builder(Plot::polar([areas, lines])).build().unwrap()
    };
    assert_eq!(scene(&chart(rows)), scene(&chart(sorted)));
}

/// Whether each vertex of each line and area of the first plot is defined.
fn defined(evaluated: &EvaluatedChart) -> Vec<Vec<bool>> {
    (plot_group(evaluated).marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Line(line) => Some(line.defined.as_vec(line.len as usize, None)),
            SceneMark::Area(area) => Some(area.defined.as_vec(area.len as usize, None)),
            _ => None,
        })
        .collect()
}

/// A row whose position is `NULL`, such as a row that `impute(method => 'gap')` adds, breaks a
/// line and an area there, and a series without one is defined everywhere.
#[test]
fn a_null_position_breaks_a_line_and_an_area() {
    let rows = RecordBatch::try_from_iter([
        (
            "key",
            Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0])) as ArrayRef,
        ),
        (
            "value",
            Arc::new(Float64Array::from(vec![
                Some(1.0),
                None,
                Some(2.0),
                Some(3.0),
            ])),
        ),
    ])
    .unwrap();
    let gap = vec![vec![true, false, true, true]];
    let evaluated = evaluate(
        &line(rows.clone(), LineMark::builder().x("key").y("value")),
        [320.0, 240.0],
    );
    assert_eq!(defined(&evaluated), gap);
    let evaluated = evaluate(
        &area(rows, AreaMark::builder().x("key").y("value")),
        [320.0, 240.0],
    );
    assert_eq!(defined(&evaluated), gap);
    let (dense, _) = shuffled();
    let evaluated = evaluate(
        &line(dense, LineMark::builder().x("key").y("value")),
        [320.0, 240.0],
    );
    assert_eq!(defined(&evaluated), vec![vec![true; 5]]);
}

/// A polar line breaks at a row whose radius is `NULL`, and every vertex that it draws has a
/// finite pixel.
#[test]
fn a_null_radius_breaks_a_polar_line() {
    let rows = RecordBatch::try_from_iter([
        (
            "key",
            Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0])) as ArrayRef,
        ),
        (
            "value",
            Arc::new(Float64Array::from(vec![
                Some(1.0),
                None,
                Some(2.0),
                Some(3.0),
            ])),
        ),
    ])
    .unwrap();
    let chart = Chart::builder(Plot::polar([PolarLayer::line(
        rows,
        PolarLineMark::builder().theta("key").radius("value"),
    )]))
    .build()
    .unwrap();
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let lines: Vec<_> = (plot_group(&evaluated).marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Line(line) => Some(line),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 1);
    let line = lines[0];
    let len = line.len as usize;
    let vertices: Vec<_> = (line.x.as_vec(len, None).into_iter())
        .zip(line.y.as_vec(len, None))
        .zip(line.defined.as_vec(len, None))
        .collect();
    assert!(vertices.iter().any(|(_, defined)| !defined), "{vertices:?}");
    assert!(
        (vertices.iter())
            .filter(|(_, defined)| *defined)
            .all(|((x, y), _)| x.is_finite() && y.is_finite()),
        "{vertices:?}"
    );
}

/// A polar area breaks at a row whose radius is `NULL` into a region for each run of the other
/// rows, and its outline breaks there too.
#[test]
fn a_null_radius_breaks_a_polar_area_into_regions() {
    let rows = RecordBatch::try_from_iter([
        (
            "key",
            Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0])) as ArrayRef,
        ),
        (
            "value",
            Arc::new(Float64Array::from(vec![
                Some(1.0),
                Some(2.0),
                None,
                Some(2.0),
                Some(3.0),
            ])),
        ),
    ])
    .unwrap();
    let chart = Chart::builder(Plot::polar([PolarLayer::area(
        rows,
        PolarAreaMark::builder()
            .theta("key")
            .radius("value")
            .stroke_width(1.0),
    )]))
    .build()
    .unwrap();
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let marks = &plot_group(&evaluated).marks;
    let regions: Vec<usize> = (marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Path(path) => Some(
                (path.path.as_vec(1, None)[0].iter())
                    .filter(|event| matches!(event, lyon_path::Event::Begin { .. }))
                    .count(),
            ),
            _ => None,
        })
        .collect();
    assert_eq!(regions, vec![2]);
    let breaks: Vec<usize> = (marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Line(line) => Some(
                (line.defined.as_vec(line.len as usize, None).into_iter())
                    .filter(|defined| !defined)
                    .count(),
            ),
            _ => None,
        })
        .collect();
    assert_eq!(breaks, vec![1]);
}
