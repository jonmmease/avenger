//! A pixel column's M4 samples become the line's points.

use m4_series::m4::column_points;

/// A column gives its four samples in x order, whatever order the aggregates name them in.
#[test]
fn a_column_gives_its_samples_in_x_order() {
    let (first, last, low, high) = ([10.0, 1.0], [13.0, 2.0], [12.0, -4.0], [11.0, 5.0]);
    assert_eq!(
        column_points([first, last, low, high]),
        [first, high, low, last]
    );
}

/// A column whose lowest or highest sample is its first or last gives that sample once.
#[test]
fn a_column_gives_each_sample_once() {
    let (first, last) = ([10.0, 1.0], [13.0, 6.0]);
    assert_eq!(column_points([first, last, first, last]), [first, last]);
    let only = [7.0, 3.0];
    assert_eq!(column_points([only; 4]), [only]);
}
