//! The projection of a polar plot: angles clockwise from the top and distances from the center.

use avenger_coords::{PlotProjection, PolarProjection};

/// The projection of a circle about `[100, 100]` from a radius of 20 to 80, between `angles`.
fn polar(angles: [f32; 2]) -> PolarProjection {
    PolarProjection::new([100.0, 100.0], [20.0, 80.0], angles)
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

/// Assert that each point of `path` lies on the circle of `radius` about `[100, 100]`, and that
/// the middle of each step lies within a quarter of a pixel of the circle.
fn follows_circle(path: &[[f32; 2]], radius: f32) {
    for [x, y] in path {
        assert!(((x - 100.0).hypot(y - 100.0) - radius).abs() < 1e-3);
    }
    for pair in path.windows(2) {
        let [x, y] = [0, 1].map(|axis| (pair[0][axis] + pair[1][axis]) / 2.0);
        let sag = radius - (x - 100.0).hypot(y - 100.0);
        assert!(sag <= 0.25 + 1e-3, "{sag}");
    }
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
fn paths_follow_arcs_within_a_quarter_of_a_pixel_and_spokes_straight() {
    let projection = polar([0.0, 360.0]);
    let arc = projection.path([0.0, 50.0], [90.0, 50.0]);
    assert_eq!(arc.len(), 1);
    follows_circle(&arc[0], 50.0);
    // An arc at a larger radius bends farther from its steps, so it takes more of them.
    let wide = projection.path([0.0, 80.0], [90.0, 80.0]).concat();
    follows_circle(&wide, 80.0);
    assert!(wide.len() > arc[0].len());
    // A spoke at one angle is straight.
    let spoke = projection.path([90.0, 20.0], [90.0, 80.0]).concat();
    let ([x0, y0], [x1, y1]) = (spoke[0], spoke[spoke.len() - 1]);
    assert!((x0 - 120.0).abs() < 1e-3 && (x1 - 180.0).abs() < 1e-3);
    assert!((y0 - 100.0).abs() < 1e-3 && (y1 - 100.0).abs() < 1e-3);
    assert!(spoke.iter().all(|[_, y]| (y - 100.0).abs() < 1e-3));
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

/// A clamp keeps a radius within the ring, keeps an angle within a part of the circle, and leaves
/// the angle of a whole circle, which wraps, as it is.
#[test]
fn a_clamp_leaves_a_wrapping_angle_as_it_is() {
    let whole = polar([0.0, 360.0]);
    assert_eq!(whole.clamp([400.0, 95.0]), [400.0, 80.0]);
    let part = polar([0.0, 90.0]);
    assert_eq!(part.clamp([120.0, 10.0]), [90.0, 20.0]);
}

/// The signed area of `ring` in pixels, positive for a ring wound clockwise on the screen.
fn area(ring: &[[f32; 2]]) -> f32 {
    (0..ring.len())
        .map(|index| {
            let ([x0, y0], [x1, y1]) = (ring[index], ring[(index + 1) % ring.len()]);
            x0 * y1 - x1 * y0
        })
        .sum::<f32>()
        / 2.0
}

/// A polar plot's tangents are the clockwise and outward directions, also at its center, and
/// agree with the pixels' finite difference.
#[test]
fn polar_tangents_are_clockwise_and_outward() {
    let projection = polar([0.0, 360.0]);
    let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3;
    let [theta, radius] = projection.tangents([90.0, 50.0]).unwrap();
    assert!(close(theta, [0.0, 1.0]) && close(radius, [1.0, 0.0]));
    let [theta, radius] = projection.tangents([0.0, 0.0]).unwrap();
    assert!(close(theta, [1.0, 0.0]) && close(radius, [0.0, -1.0]));
    #[derive(Debug)]
    struct Pixels(PolarProjection);
    impl PlotProjection for Pixels {
        fn position(&self, pixel: [f32; 2]) -> Option<[f64; 2]> {
            self.0.position(pixel)
        }
        fn pixel(&self, position: [f64; 2]) -> Option<[f32; 2]> {
            self.0.pixel(position)
        }
        fn extent(&self) -> [[f64; 2]; 2] {
            self.0.extent()
        }
        fn period(&self, dimension: usize) -> Option<f64> {
            self.0.period(dimension)
        }
    }
    for position in [[30.0, 40.0], [200.0, 70.0], [359.0, 80.0]] {
        let [a, b] = projection.tangents(position).unwrap();
        let [c, d] = Pixels(projection).tangents(position).unwrap();
        assert!(close(a, c) && close(b, d), "{position:?}");
    }
}

/// A whole circle with a hole outlines its outer circle and its inner circle wound the other way,
/// and a part of a circle without one outlines one ring through the center. The bounds are the
/// arcs' box exactly.
#[test]
fn polar_outlines_and_bounds_follow_the_arcs() {
    let rings = polar([0.0, 360.0]).outline();
    assert_eq!(rings.len(), 2);
    assert!(area(&rings[0]) * area(&rings[1]) < 0.0);
    assert!((area(&rings[0]).abs() - std::f32::consts::PI * 80.0 * 80.0).abs() < 50.0);
    assert_eq!(
        polar([0.0, 360.0]).bounds(),
        Some([[20.0, 20.0], [180.0, 180.0]])
    );
    let quarter = PolarProjection::new([100.0, 100.0], [0.0, 80.0], [0.0, 90.0]);
    let rings = quarter.outline();
    assert_eq!(rings.len(), 1);
    assert_eq!(rings[0].last(), Some(&[100.0, 100.0]));
    let [[x0, y0], [x1, y1]] = quarter.bounds().unwrap();
    assert!((x0 - 100.0).abs() < 1e-3 && (y0 - 20.0).abs() < 1e-3);
    assert!((x1 - 180.0).abs() < 1e-3 && (y1 - 100.0).abs() < 1e-3);
}

/// A part of a circle shows its sector, a route runs the short way around a whole circle, and an
/// iso-line of a radius rings the whole circle.
#[test]
fn polar_regions_routes_and_iso_lines() {
    let part = polar([0.0, 90.0]);
    assert!(part.shows([45.0, 50.0]));
    assert!(!part.shows([135.0, 50.0]));
    assert!(!part.shows([45.0, 90.0]));
    let route = polar([0.0, 360.0])
        .route([350.0, 50.0], [10.0, 50.0])
        .concat();
    follows_circle(&route, 50.0);
    assert!(route
        .iter()
        .any(|[x, y]| (x - 100.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3));
    let ring = polar([0.0, 360.0]).iso_line(1, 50.0, None).concat();
    follows_circle(&ring, 50.0);
    let ([x0, y0], [x1, y1]) = (ring[0], ring[ring.len() - 1]);
    assert!((x0 - x1).abs() < 1e-3 && (y0 - y1).abs() < 1e-3);
    let spoke = polar([0.0, 360.0]).iso_line(0, 90.0, None).concat();
    let ([x0, y0], [x1, y1]) = (spoke[0], spoke[spoke.len() - 1]);
    assert!((x0 - 120.0).abs() < 1e-3 && (y0 - 100.0).abs() < 1e-3);
    assert!((x1 - 180.0).abs() < 1e-3 && (y1 - 100.0).abs() < 1e-3);
}
