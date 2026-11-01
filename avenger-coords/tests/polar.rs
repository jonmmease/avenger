//! The projection of a polar plot: angles clockwise from the top and distances from the center.

use avenger_coords::{PlotProjection, PolarProjection};

/// The projection of a circle about `[100, 100]` from a radius of 20 to 80, between `angles`.
fn polar(angles: [f32; 2]) -> PolarProjection {
    PolarProjection::new([100.0, 100.0], [20.0, 80.0], angles)
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

#[test]
fn positions_are_angles_clockwise_from_the_top_and_distances() {
    let projection = polar([0.0, 360.0]);
    assert!(close(
        projection.position([100.0, 40.0]).unwrap(),
        [0.0, 60.0]
    ));
    assert!(close(
        projection.position([150.0, 100.0]).unwrap(),
        [90.0, 50.0]
    ));
    assert!(close(
        projection.position([100.0, 130.0]).unwrap(),
        [180.0, 30.0]
    ));
    assert!(close(
        projection.position([60.0, 100.0]).unwrap(),
        [270.0, 40.0]
    ));
    let pixel = projection.pixel([90.0, 50.0]).unwrap();
    assert!((pixel[0] - 150.0).abs() < 1e-3 && (pixel[1] - 100.0).abs() < 1e-3);
}

#[test]
fn angles_lie_within_half_a_turn_of_the_middle_of_the_part() {
    let projection = polar([-90.0, 90.0]);
    // The left of the circle is -90 degrees, the start of the half circle, not 270.
    assert!(close(
        projection.position([60.0, 100.0]).unwrap(),
        [-90.0, 40.0]
    ));
    // Angles past the half circle lie past its nearer end, where a tool clamps them: 135 degrees
    // at the lower right, and -135 at the lower left.
    assert!(close(
        projection.position([130.0, 130.0]).unwrap(),
        [135.0, 30.0_f64.hypot(30.0)]
    ));
    assert!(close(
        projection.position([70.0, 130.0]).unwrap(),
        [-135.0, 30.0_f64.hypot(30.0)]
    ));
}

#[test]
fn a_whole_circle_wraps_its_angle() {
    let whole = polar([0.0, 360.0]);
    assert_eq!(whole.period(0), Some(360.0));
    assert_eq!(whole.period(1), None);
    assert_eq!(whole.extent(), [[0.0, 360.0], [20.0, 80.0]]);
    let half = polar([-90.0, 90.0]);
    assert_eq!(half.period(0), None);
    assert_eq!(half.extent(), [[-90.0, 90.0], [20.0, 80.0]]);
}

#[test]
fn paths_sweep_arcs_and_run_straight_along_spokes() {
    let projection = polar([0.0, 360.0]);
    // An arc at one radius takes a point every 2 degrees, each at the radius.
    let arc = projection.path([0.0, 50.0], [90.0, 50.0]);
    assert_eq!(arc.len(), 1);
    assert_eq!(arc[0].len(), 46);
    for [x, y] in &arc[0] {
        assert!(((x - 100.0).hypot(y - 100.0) - 50.0).abs() < 1e-3);
    }
    // A spoke at one angle is a straight segment.
    let spoke = projection.path([90.0, 20.0], [90.0, 80.0]);
    assert_eq!(spoke, vec![vec![[120.0, 100.0], [180.0, 100.0]]]);
    // A range across the wrap sweeps through the top.
    let across = projection.path([350.0, 50.0], [370.0, 50.0]);
    let top = across[0][5];
    assert!((top[0] - 100.0).abs() < 1e-3 && (top[1] - 50.0).abs() < 1e-3);
}

/// Whether the polygon `ring` holds `point`, by the even-odd rule.
fn holds(ring: &[[f64; 2]], [x, y]: [f64; 2]) -> bool {
    let mut inside = false;
    for (index, &[xi, yi]) in ring.iter().enumerate() {
        let [xj, yj] = ring[(index + ring.len() - 1) % ring.len()];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
    }
    inside
}

#[test]
fn a_lasso_across_the_top_splits_at_the_wrap() {
    let projection = polar([0.0, 360.0]);
    // A triangle over the top of the circle, from about -11 to 11 degrees.
    let path = [[90.0, 50.0], [110.0, 50.0], [100.0, 70.0]];
    let polygons = projection.enclosed(&path);
    assert_eq!(polygons.len(), 2);
    let held = |point: [f64; 2]| polygons.iter().any(|ring| holds(ring, point));
    assert!(held([5.0, 45.0]) && held([355.0, 45.0]));
    assert!(!held([180.0, 45.0]) && !held([90.0, 45.0]));
}

#[test]
fn a_lasso_around_the_center_closes_through_the_hole() {
    let projection = polar([0.0, 360.0]);
    // A square around the center, 40 pixels out at its sides.
    let path = [[60.0, 60.0], [140.0, 60.0], [140.0, 140.0], [60.0, 140.0]];
    for path in [path.to_vec(), path.iter().rev().copied().collect()] {
        let polygons = projection.enclosed(&path);
        let held = |point: [f64; 2]| polygons.iter().any(|ring| holds(ring, point));
        // Every angle holds the radii from the hole to the square.
        for angle in [1.0, 45.0, 135.0, 225.0, 359.0] {
            assert!(held([angle, 30.0]), "{angle}");
            assert!(!held([angle, 70.0]), "{angle}");
        }
    }
}
