//! Spherical point-in-polygon test.
//!
//! Ported from d3-geo `src/polygonContains.js` (ISC),
//! <https://github.com/d3/d3-geo>. Polygon rings are sequences of spherical
//! radian points; the convention is the d3 spherical winding (clockwise
//! exteriors).

use crate::math::{
    asin, cartesian, cartesian_cross, cartesian_normalize_in_place, EPSILON, EPSILON2, HALF_PI, PI,
    QUARTER_PI, TAU,
};

/// Normalize a longitude into [-π, π] (d3 `longitude()`).
fn longitude(lambda: f64) -> f64 {
    if lambda.abs() <= PI {
        lambda
    } else {
        crate::math::sign(lambda) * ((lambda.abs() + PI) % TAU - PI)
    }
}

pub fn polygon_contains(polygon: &[Vec<[f64; 2]>], point: [f64; 2]) -> bool {
    let lambda = longitude(point[0]);
    let mut phi = point[1];
    let sin_phi = phi.sin();
    let normal = [lambda.sin(), -lambda.cos(), 0.0];
    let mut angle = 0.0_f64;
    let mut winding: i64 = 0;
    let mut sum = 0.0_f64;

    if sin_phi == 1.0 {
        phi = HALF_PI + EPSILON;
    } else if sin_phi == -1.0 {
        phi = -HALF_PI - EPSILON;
    }

    for ring in polygon {
        let m = ring.len();
        if m == 0 {
            continue;
        }
        let mut point0 = ring[m - 1];
        let mut lambda0 = longitude(point0[0]);
        let phi0_half = point0[1] / 2.0 + QUARTER_PI;
        let mut sin_phi0 = phi0_half.sin();
        let mut cos_phi0 = phi0_half.cos();

        for point1 in ring.iter().copied() {
            let lambda1 = longitude(point1[0]);
            let phi1_half = point1[1] / 2.0 + QUARTER_PI;
            let sin_phi1 = phi1_half.sin();
            let cos_phi1 = phi1_half.cos();
            let delta = lambda1 - lambda0;
            let sgn = if delta >= 0.0 { 1.0 } else { -1.0 };
            let abs_delta = sgn * delta;
            let antimeridian = abs_delta > PI;
            let k = sin_phi0 * sin_phi1;

            sum += (k * sgn * abs_delta.sin()).atan2(cos_phi0 * cos_phi1 + k * abs_delta.cos());
            angle += if antimeridian {
                delta + sgn * TAU
            } else {
                delta
            };

            // Are the longitudes either side of the point's meridian (lambda),
            // and are the latitudes smaller than the parallel (phi)?
            if antimeridian ^ (lambda0 >= lambda) ^ (lambda1 >= lambda) {
                let mut arc = cartesian_cross(
                    &cartesian(point0[0], point0[1]),
                    &cartesian(point1[0], point1[1]),
                );
                cartesian_normalize_in_place(&mut arc);
                let mut intersection = cartesian_cross(&normal, &arc);
                cartesian_normalize_in_place(&mut intersection);
                let phi_arc = if antimeridian ^ (delta >= 0.0) {
                    -asin(intersection[2])
                } else {
                    asin(intersection[2])
                };
                if phi > phi_arc || (phi == phi_arc && (arc[0] != 0.0 || arc[1] != 0.0)) {
                    winding += if antimeridian ^ (delta >= 0.0) { 1 } else { -1 };
                }
            }

            point0 = point1;
            lambda0 = lambda1;
            sin_phi0 = sin_phi1;
            cos_phi0 = cos_phi1;
        }
    }

    // First, determine whether the South pole is inside or outside:
    //
    // It is inside if:
    // * the polygon winds around it in a clockwise direction.
    // * the polygon does not (cumulatively) wind around it, but has a
    //   negative (counter-clockwise) area.
    //
    // Second, count the (signed) number of times a segment crosses a lambda
    // from the point to the South pole. If it is zero, then the point is the
    // same side as the South pole.
    ((angle < -EPSILON || (angle < EPSILON && sum < -EPSILON2)) as i64 ^ (winding & 1)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::RADIANS;

    fn ring_deg(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
        // d3 polygonContains input rings are closed (first == last handled by
        // iteration from last point), in spherical radians.
        points
            .iter()
            .map(|p| [p[0] * RADIANS, p[1] * RADIANS])
            .collect()
    }

    #[test]
    fn simple_square() {
        // Clockwise (spherical convention) exterior around (0.5, 0.5).
        let polygon = vec![ring_deg(&[
            [0.0, 0.0],
            [0.0, 1.0],
            [1.0, 1.0],
            [1.0, 0.0],
            [0.0, 0.0],
        ])];
        assert!(polygon_contains(&polygon, [0.5 * RADIANS, 0.5 * RADIANS]));
        assert!(!polygon_contains(&polygon, [2.0 * RADIANS, 0.5 * RADIANS]));
        assert!(!polygon_contains(&polygon, [0.5 * RADIANS, -0.5 * RADIANS]));
    }

    #[test]
    fn south_pole_cap() {
        // A parallel ring at -80° traversed EASTWARD encloses the south pole
        // (clockwise as seen from the pole — the d3 spherical convention).
        let ring: Vec<[f64; 2]> = (0..36)
            .map(|i| {
                let lon = (i as f64) * 10.0;
                [lon * RADIANS, -80.0 * RADIANS]
            })
            .collect();
        let polygon = vec![ring.clone()];
        assert!(polygon_contains(&polygon, [0.0, -89.0 * RADIANS]));
        assert!(!polygon_contains(&polygon, [0.0, 0.0]));

        // The reverse (westward) ring encloses the complement.
        let reversed: Vec<[f64; 2]> = ring.into_iter().rev().collect();
        let polygon = vec![reversed];
        assert!(!polygon_contains(&polygon, [0.0, -89.0 * RADIANS]));
        assert!(polygon_contains(&polygon, [0.0, 0.0]));
    }
}
