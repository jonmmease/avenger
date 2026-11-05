//! Plot coordinates: conversions between data and positions through a projection and the scales
//! of its dimensions.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, Float64Array, Int64Array, StringArray};
use avenger_coords::{PlotCoords, PlotProjection, PolarProjection};
use avenger_scales::scales::linear::LinearScale;

/// A Cartesian plot 200 px wide and 100 px high, with x from 0 to 10 and y from 0 to 5 up the plot.
fn cartesian() -> PlotCoords {
    PlotCoords::cartesian(
        [200.0, 100.0],
        [
            Some(LinearScale::configured((0.0, 10.0), (0.0, 200.0))),
            Some(LinearScale::configured((0.0, 5.0), (100.0, 0.0))),
        ],
    )
}

/// Each dimension converts through its scale, in the direction of the scale's range, and a
/// dimension without a scale takes its data as its positions.
#[test]
fn coords_convert_data_through_the_scale_of_each_dimension() {
    let coords = cartesian();
    assert_eq!(coords.position(0, 5.0).unwrap(), 100.0);
    assert_eq!(coords.value(0, 50.0).unwrap(), 2.5);
    // The y scale runs up the plot, so its largest value is at the top.
    assert_eq!(coords.position(1, 5.0).unwrap(), 0.0);
    assert_eq!(coords.value(1, 100.0).unwrap(), 0.0);
    let unscaled = PlotCoords::cartesian([200.0, 100.0], [None, None]);
    assert_eq!(unscaled.position(0, 42.0).unwrap(), 42.0);
    assert_eq!(unscaled.value(1, 42.0).unwrap(), 42.0);
}

/// A polar plot's scales map data to angles and to distances from the center.
#[test]
fn polar_coords_convert_angles_and_radii() {
    let projection = PolarProjection::new([100.0, 100.0], [0.0, 100.0], [0.0, 360.0]);
    let theta = LinearScale::configured((0.0, 12.0), (0.0, 360.0));
    let radius = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
    let coords = PlotCoords::new(Arc::new(projection), [Some(theta), Some(radius)]);
    assert_eq!(coords.position(0, 3.0).unwrap(), 90.0);
    assert_eq!(coords.value(1, 50.0).unwrap(), 5.0);
}

/// A closed path in pixels encloses a polygon of data.
#[test]
fn coords_enclose_polygons_of_data() {
    let path = [[20.0, 20.0], [180.0, 20.0], [180.0, 80.0], [20.0, 80.0]];
    let polygons = cartesian().enclosed(&path);
    assert_eq!(polygons.len(), 1);
    let bounds = |axis: usize| {
        let values = polygons[0].iter().map(|vertex| vertex[axis]);
        let low = values.clone().fold(f64::INFINITY, f64::min);
        (low, values.fold(f64::NEG_INFINITY, f64::max))
    };
    let near =
        |(a, b): (f64, f64), (c, d): (f64, f64)| (a - c).abs() < 1e-9 && (b - d).abs() < 1e-9;
    assert!(near(bounds(0), (1.0, 9.0)), "{:?}", bounds(0));
    assert!(near(bounds(1), (1.0, 4.0)), "{:?}", bounds(1));
}

/// The values of `pixels`, with `None` for a null.
fn values(pixels: &Float32Array) -> Vec<Option<f32>> {
    pixels.iter().collect()
}

/// A batch of rows converts through each dimension's scale to pixels, and a null in either
/// dimension leaves the row without a pixel.
#[test]
fn coords_place_a_batch_of_rows_through_their_scales() {
    let x: ArrayRef = Arc::new(Float64Array::from(vec![Some(0.0), Some(5.0), None]));
    let y: ArrayRef = Arc::new(Float64Array::from(vec![Some(5.0), Some(0.0), Some(1.0)]));
    let [px, py] = cartesian().pixels([&x, &y]).unwrap();
    assert_eq!(values(&px), [Some(0.0), Some(100.0), None]);
    assert_eq!(values(&py), [Some(0.0), Some(100.0), None]);
}

/// A dimension without a scale takes a numeric column of any type as its positions.
#[test]
fn coords_place_unscaled_integers() {
    let coords = PlotCoords::cartesian(
        [200.0, 100.0],
        [
            Some(LinearScale::configured((0.0, 10.0), (0.0, 200.0))),
            None,
        ],
    );
    let x: ArrayRef = Arc::new(Float64Array::from(vec![5.0]));
    let y: ArrayRef = Arc::new(Int64Array::from(vec![42]));
    let [px, py] = coords.pixels([&x, &y]).unwrap();
    assert_eq!(values(&px), [Some(100.0)]);
    assert_eq!(values(&py), [Some(42.0)]);
}

/// A dimension without a scale rejects a column that is not numeric, and the two columns must
/// have one length.
#[test]
fn coords_reject_unscaled_text_and_columns_of_two_lengths() {
    let unscaled = PlotCoords::cartesian([200.0, 100.0], [None, None]);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1"]));
    let one: ArrayRef = Arc::new(Float64Array::from(vec![1.0]));
    let two: ArrayRef = Arc::new(Float64Array::from(vec![1.0, 2.0]));
    assert!(unscaled.pixels([&text, &one]).is_err());
    assert!(unscaled.pixels([&one, &two]).is_err());
}

/// A polar batch places each row where the projection places its position.
#[test]
fn polar_coords_place_a_batch_as_each_position() {
    let projection = PolarProjection::new([100.0, 100.0], [0.0, 100.0], [0.0, 360.0]);
    let theta = LinearScale::configured((0.0, 12.0), (0.0, 360.0));
    let radius = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
    let coords = PlotCoords::new(Arc::new(projection), [Some(theta), Some(radius)]);
    let months = [0.0, 3.0, 7.5];
    let amounts = [10.0, 5.0, 2.0];
    let x: ArrayRef = Arc::new(Float64Array::from(months.to_vec()));
    let y: ArrayRef = Arc::new(Float64Array::from(amounts.to_vec()));
    let [px, py] = coords.pixels([&x, &y]).unwrap();
    for (row, (month, amount)) in months.iter().zip(amounts).enumerate() {
        let position = [
            coords.position(0, *month).unwrap(),
            coords.position(1, amount).unwrap(),
        ];
        let [ex, ey] = projection.pixel(position).unwrap();
        assert!(
            (px.value(row) - ex).abs() < 1e-3 && (py.value(row) - ey).abs() < 1e-3,
            "row {row}"
        );
    }
}

/// A whole circle of twelve months around its theta axis, and values from 0 to 10 outward.
fn months() -> PlotCoords {
    let projection = PolarProjection::new([100.0, 100.0], [0.0, 100.0], [0.0, 360.0]);
    let theta = LinearScale::configured((0.0, 12.0), (0.0, 360.0));
    let radius = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
    PlotCoords::new(Arc::new(projection), [Some(theta), Some(radius)])
}

/// A dimension's view knows its direction and its period in data, and turns a range of data
/// across the wrap into positions and back.
#[test]
fn dimensions_convert_ranges_across_the_wrap() {
    let coords = cartesian();
    assert!(coords.dimension(0).increasing());
    assert!(!coords.dimension(1).increasing());
    assert_eq!(coords.dimension(0).data_period(), None);
    let theta = months();
    let theta = theta.dimension(0);
    assert_eq!(theta.data_period(), Some(12.0));
    assert_eq!(theta.short(350.0), -10.0);
    assert_eq!(theta.whole(), [0.0, 360.0]);
    let positions = theta.positions_of([11.0, 1.0]).unwrap();
    assert!((positions[0] - 330.0).abs() < 1e-3 && (positions[1] - 390.0).abs() < 1e-3);
    let values = theta.values_of(positions).unwrap();
    assert!((values[0] - 11.0).abs() < 1e-6 && (values[1] - 1.0).abs() < 1e-6);
    let unscaled = PlotCoords::new(
        Arc::new(PolarProjection::new([0.0, 0.0], [0.0, 1.0], [0.0, 360.0])),
        [None, None],
    );
    assert_eq!(unscaled.dimension(0).data_period(), Some(360.0));
}

/// Coordinates contain a pixel where the plot shows a position, and their positions are Float32
/// through a scale and Float64 without one.
#[test]
fn coords_contain_shown_pixels_and_give_positions() {
    let coords = months();
    assert!(coords.contains([100.0, 50.0]));
    assert!(!coords.contains([5.0, 5.0]));
    let x: ArrayRef = Arc::new(Float64Array::from(vec![3.0]));
    let y: ArrayRef = Arc::new(Int64Array::from(vec![5]));
    let unscaled_y = PlotCoords::new(
        coords.projection().clone(),
        [coords.scale(0).cloned(), None],
    );
    let [theta, radius] = unscaled_y.positions([&x, &y]).unwrap();
    assert_eq!(theta.data_type(), &arrow::datatypes::DataType::Float32);
    assert_eq!(radius.data_type(), &arrow::datatypes::DataType::Float64);
}
