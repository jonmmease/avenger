//! Plot projections: the Cartesian projection and the default paths, polygons, and enclosed
//! paths of a projection.

use arrow::array::{Float32Array, Float64Array};
use avenger_coords::{pieces, short_way, CartesianProjection, PlotProjection};

/// A projection that scales positions by 10 and shows no position at an x of 5 or more, to test the
/// defaults that resample.
#[derive(Debug)]
struct Hidden;

impl PlotProjection for Hidden {
    fn position(&self, pixel: [f32; 2]) -> Option<[f64; 2]> {
        (pixel[0] < 50.0).then(|| pixel.map(|value| f64::from(value) / 10.0))
    }

    fn pixel(&self, position: [f64; 2]) -> Option<[f32; 2]> {
        (position[0] < 5.0).then(|| position.map(|value| (value * 10.0) as f32))
    }

    fn extent(&self) -> [[f64; 2]; 2] {
        [[0.0, 10.0], [0.0, 10.0]]
    }
}

#[test]
fn cartesian_positions_are_pixels() {
    let cartesian = CartesianProjection::new([200.0, 100.0]);
    assert_eq!(cartesian.position([20.0, 30.0]), Some([20.0, 30.0]));
    assert_eq!(cartesian.pixel([20.0, 30.0]), Some([20.0, 30.0]));
    // A pixel outside the rectangle has a position, which a tool clamps to the extent.
    assert_eq!(cartesian.position([-5.0, 130.0]), Some([-5.0, 130.0]));
    assert_eq!(cartesian.extent(), [[0.0, 200.0], [0.0, 100.0]]);
    assert_eq!(cartesian.period(0), None);
    assert!(cartesian.is_linear());
}

#[test]
fn cartesian_paths_and_polygons_are_straight() {
    let cartesian = CartesianProjection::new([200.0, 100.0]);
    assert_eq!(
        cartesian.path([10.0, 10.0], [50.0, 90.0]),
        vec![vec![[10.0, 10.0], [50.0, 90.0]]]
    );
    let ring = [[10.0, 10.0], [50.0, 10.0], [50.0, 90.0]];
    assert_eq!(
        cartesian.polygon(&ring),
        vec![vec![[10.0, 10.0], [50.0, 10.0], [50.0, 90.0]]]
    );
    assert!(cartesian.polygon(&ring[..2]).is_empty());
}

#[test]
fn default_path_resamples_and_breaks_where_hidden() {
    let path = Hidden.path([0.0, 0.0], [8.0, 0.0]);
    // The positions past an x of 5 are hidden, and the path halves its step across them, so one
    // piece runs from the start to within a thousandth of the path of the cut.
    assert_eq!(path.len(), 1);
    let piece = &path[0];
    assert_eq!(piece[0], [0.0, 0.0]);
    let last = piece[piece.len() - 1];
    assert!(last[0] < 50.0 && last[0] > 49.9, "{last:?}");
    assert!(piece.windows(2).all(|pair| pair[1][0] > pair[0][0]));
}

#[test]
fn default_polygon_joins_the_sides() {
    let ring = [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
    let rings = Hidden.polygon(&ring);
    assert_eq!(rings.len(), 1);
    let ring = &rings[0];
    // Each side takes the four steps that a path starts from, which are straight, so none halves,
    // and each corner appears once.
    assert_eq!(ring.len(), 4 * 4);
    assert_eq!(ring[0], [0.0, 0.0]);
    assert_eq!(ring[4], [40.0, 0.0]);
}

/// A projection whose first dimension wraps every 10 positions, one pixel to a position.
#[derive(Debug)]
struct Wrapped;

impl PlotProjection for Wrapped {
    fn position(&self, pixel: [f32; 2]) -> Option<[f64; 2]> {
        Some([f64::from(pixel[0]).rem_euclid(10.0), f64::from(pixel[1])])
    }

    fn pixel(&self, position: [f64; 2]) -> Option<[f32; 2]> {
        Some(position.map(|value| value as f32))
    }

    fn extent(&self) -> [[f64; 2]; 2] {
        [[0.0, 10.0], [0.0, 10.0]]
    }

    fn period(&self, dimension: usize) -> Option<f64> {
        (dimension == 0).then_some(10.0)
    }
}

#[test]
fn polygon_sides_run_the_short_way_around() {
    assert_eq!(short_way(&Wrapped, [9.0, 0.0], [1.0, 5.0]), [11.0, 5.0]);
    assert_eq!(short_way(&Wrapped, [1.0, 0.0], [9.0, 5.0]), [-1.0, 5.0]);
    assert_eq!(
        short_way(
            &CartesianProjection::new([10.0, 10.0]),
            [9.0, 0.0],
            [1.0, 5.0]
        ),
        [1.0, 5.0]
    );
    // The side from 9 to 1 crosses the wrap to 11, and the side from 1 to 9 crosses it to -1,
    // rather than running back through 5.
    let rings = Wrapped.polygon(&[[9.0, 0.0], [1.0, 0.0], [1.0, 4.0], [9.0, 4.0]]);
    assert_eq!(rings.len(), 1);
    assert!(rings[0]
        .iter()
        .all(|[x, _]| *x <= 1.0 + 1e-6 || *x >= 9.0 - 1e-6));
}

#[test]
fn pieces_break_at_gaps_and_drop_single_pixels() {
    let pixels = [
        Some([0.0, 0.0]),
        Some([1.0, 0.0]),
        None,
        Some([3.0, 0.0]),
        None,
        Some([5.0, 0.0]),
        Some([6.0, 0.0]),
    ];
    assert_eq!(
        pieces(pixels),
        vec![vec![[0.0, 0.0], [1.0, 0.0]], vec![[5.0, 0.0], [6.0, 0.0]]]
    );
}

#[test]
fn enclosed_paths_convert_their_vertices() {
    let cartesian = CartesianProjection::new([200.0, 100.0]);
    let path = [[10.0, 10.0], [50.0, 10.0], [50.0, 90.0]];
    assert_eq!(
        cartesian.enclosed(&path),
        vec![vec![[10.0, 10.0], [50.0, 10.0], [50.0, 90.0]]]
    );
    // A projection that is not linear resamples the path first, and gives no polygon where a vertex has no
    // position.
    let shown = [[0.0, 0.0], [40.0, 0.0], [40.0, 40.0]];
    let ring = &Hidden.enclosed(&shown)[0];
    assert_eq!(ring.len(), 5 + 5 + 8);
    let hidden = [[0.0, 0.0], [80.0, 0.0], [80.0, 40.0]];
    assert!(Hidden.enclosed(&hidden).is_empty());
}

#[test]
fn enclosed_polygons_split_at_the_wrap() {
    // A square from x = 8 to 12 runs across the wrap at 10, so it splits into a polygon on each
    // side of it, each within the first period, and a millionth of a period past it.
    let path = [[8.0, 0.0], [12.0, 0.0], [12.0, 4.0], [8.0, 4.0]];
    let mut polygons = Wrapped.enclosed(&path);
    polygons.sort_by(|a, b| a[0][0].total_cmp(&b[0][0]));
    assert_eq!(polygons.len(), 2);
    let bounds = |ring: &[[f64; 2]]| {
        let xs = ring.iter().map(|[x, _]| *x);
        [
            xs.clone().fold(f64::MAX, f64::min),
            xs.fold(f64::MIN, f64::max),
        ]
    };
    let [left, right] = [bounds(&polygons[0]), bounds(&polygons[1])];
    assert!(
        (left[0] + 1e-5).abs() < 1e-7 && (left[1] - 2.0).abs() < 1e-7,
        "{left:?}"
    );
    assert!(
        (right[0] - 8.0).abs() < 1e-7 && (right[1] - 10.00001).abs() < 1e-7,
        "{right:?}"
    );
    // A square within one period stays one polygon.
    let inside = [[2.0, 0.0], [6.0, 0.0], [6.0, 4.0], [2.0, 4.0]];
    assert_eq!(
        Wrapped.enclosed(&inside),
        vec![inside.map(|p| p.map(f64::from)).to_vec()]
    );
}

/// A Cartesian plot shows the box of its extent, and a gesture past its edge stays on the edge.
#[test]
fn the_default_region_is_the_box_of_the_extent() {
    let projection = CartesianProjection::new([200.0, 100.0]);
    assert_eq!(
        projection.region(),
        [[0.0, 0.0], [200.0, 0.0], [200.0, 100.0], [0.0, 100.0]]
    );
    assert_eq!(projection.clamp([-15.0, 40.0]), [0.0, 40.0]);
    assert_eq!(projection.clamp([250.0, 130.0]), [200.0, 100.0]);
    assert_eq!(projection.clamp([120.0, 60.0]), [120.0, 60.0]);
}

/// A Cartesian projection's pixels are its positions: a Float32 dimension keeps its buffer, a
/// Float64 one narrows, and a null in either dimension nulls the row.
#[test]
fn cartesian_pixels_reuse_float32_positions() {
    let x = Float32Array::from(vec![1.0, 2.0]);
    let y = Float64Array::from(vec![Some(3.0), None]);
    let [px, py] = CartesianProjection::new([10.0, 10.0]).pixels([(&x).into(), (&y).into()]);
    assert_eq!(px.values().as_ptr(), x.values().as_ptr());
    assert_eq!(px.iter().collect::<Vec<_>>(), [Some(1.0), None]);
    assert_eq!(py.iter().collect::<Vec<_>>(), [Some(3.0), None]);
}

/// The default pixels are null where the plot shows no position.
#[test]
fn default_pixels_are_null_where_the_plot_shows_no_position() {
    let x = Float64Array::from(vec![1.0, 7.0]);
    let y = Float64Array::from(vec![2.0, 2.0]);
    let [px, py] = Hidden.pixels([(&x).into(), (&y).into()]);
    assert_eq!(px.iter().collect::<Vec<_>>(), [Some(10.0), None]);
    assert_eq!(py.iter().collect::<Vec<_>>(), [Some(20.0), None]);
}

/// A Cartesian projection shows its rectangle, outlines it, and runs its iso-lines and routes
/// straight along the pixel axes.
#[test]
fn cartesian_regions_tangents_and_lines_follow_the_rectangle() {
    let projection = CartesianProjection::new([200.0, 100.0]);
    assert!(projection.shows([10.0, 10.0]));
    assert!(projection.shows([200.0, 100.0]));
    assert!(!projection.shows([250.0, 10.0]));
    assert_eq!(
        projection.outline(),
        [vec![[0.0, 0.0], [200.0, 0.0], [200.0, 100.0], [0.0, 100.0]]]
    );
    assert_eq!(projection.bounds(), Some([[0.0, 0.0], [200.0, 100.0]]));
    assert_eq!(
        projection.tangents([50.0, 50.0]),
        Some([[1.0, 0.0], [0.0, 1.0]])
    );
    assert_eq!(
        projection.iso_line(0, 50.0, None),
        [vec![[50.0, 0.0], [50.0, 100.0]]]
    );
    assert_eq!(
        projection.iso_line(1, 20.0, Some([10.0, 30.0])),
        [vec![[10.0, 20.0], [30.0, 20.0]]]
    );
    assert_eq!(
        projection.route([0.0, 0.0], [20.0, 10.0]),
        [vec![[0.0, 0.0], [20.0, 10.0]]]
    );
}

/// The default tangents take a finite difference of the pixels, which steps inward at the high
/// end of an extent, and give none where the plot shows no position.
#[test]
fn default_tangents_follow_the_pixels() {
    let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
    let [x, y] = Hidden.tangents([1.0, 1.0]).unwrap();
    assert!(close(x, [1.0, 0.0]) && close(y, [0.0, 1.0]));
    let [_, y] = Hidden.tangents([1.0, 10.0]).unwrap();
    assert!(close(y, [0.0, 1.0]));
    assert_eq!(Hidden.tangents([6.0, 1.0]), None);
}
