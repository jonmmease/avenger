//! Adaptive great-circle resampling.
//!
//! Ported from d3-geo `src/projection/resample.js` (ISC),
//! <https://github.com/d3/d3-geo>. Recursively bisects each segment along
//! the great circle, comparing the projected spherical midpoint against
//! the linear midpoint, until the perpendicular deviation is below
//! `delta2` (squared pixel tolerance in projected output units).
//!
//! Unlike d3, the stage can know the rectangle that clips its output. A
//! piece of a segment that lies far beyond one side of the rectangle stops
//! bisecting, since the rectangle clip removes it. A map zoomed in far would
//! otherwise bisect every line on the visible sphere to the full depth,
//! though nearly all of it lands outside the rectangle.

use crate::math::{asin, cartesian, cartesian_cross, cartesian_dot, EPSILON, RADIANS};
use crate::stream::GeoSink;

const MAX_DEPTH: u32 = 16;

fn cos_min_distance() -> f64 {
    (30.0 * RADIANS).cos() // cos(minimum angular distance)
}

/// Whether the great-circle arc from `a` to `b`, shorter than a half turn and with
/// `cos_length` the cosine of its length, stays farther than its length from the poles and
/// from the antipode of the projection's center. Near those points, projections tear or bend
/// sharply: poles become points or lines, and an azimuthal projection's antipode becomes its
/// edge.
fn clear_of_tears(a: [f64; 3], b: [f64; 3], cos_length: f64) -> bool {
    let normal = cartesian_cross(&a, &b);
    let normal2 = cartesian_dot(&normal, &normal);
    [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [-1.0, 0.0, 0.0]]
        .iter()
        .all(|tear| {
            // The cosine of the arc's distance from the tear: from the great circle's nearest
            // point when it lies between the ends, else from the nearer end.
            let ends = cartesian_dot(tear, &a).max(cartesian_dot(tear, &b));
            let along = cartesian_dot(tear, &normal);
            let nearest = std::array::from_fn(|i| tear[i] - normal[i] * along / normal2);
            let between = cartesian_dot(&cartesian_cross(&a, &nearest), &normal) >= 0.0
                && cartesian_dot(&cartesian_cross(&nearest, &b), &normal) >= 0.0;
            let closest = if normal2 > 0.0 && between {
                (1.0 - along * along / normal2).max(0.0).sqrt()
            } else {
                ends
            };
            closest < cos_length
        })
}

/// The resample stage: consumes rotated spherical radians, emits projected
/// planar points. `project` is the full projection and view transform.
pub struct Resample<F, S> {
    project: F,
    delta2: f64,
    /// The rectangle `[[x0, y0], [x1, y1]]` that clips the output, beyond which pieces stop
    /// bisecting.
    cull: Option<[[f64; 2]; 2]>,
    pub sink: S,
    // First point of the current ring.
    lambda00: f64,
    x00: f64,
    y00: f64,
    a00: f64,
    b00: f64,
    c00: f64,
    // Previous point.
    lambda0: f64,
    x0: f64,
    y0: f64,
    a0: f64,
    b0: f64,
    c0: f64,
    in_line: bool,
    in_polygon: bool,
    ring_first_point: bool,
}

impl<F, S> Resample<F, S>
where
    F: Fn(f64, f64) -> (f64, f64),
    S: GeoSink,
{
    pub fn new(project: F, delta2: f64, sink: S) -> Self {
        Resample {
            project,
            delta2,
            cull: None,
            sink,
            lambda00: 0.0,
            x00: 0.0,
            y00: 0.0,
            a00: 0.0,
            b00: 0.0,
            c00: 0.0,
            lambda0: f64::NAN,
            x0: f64::NAN,
            y0: f64::NAN,
            a0: f64::NAN,
            b0: f64::NAN,
            c0: f64::NAN,
            in_line: false,
            in_polygon: false,
            ring_first_point: false,
        }
    }

    /// Stop bisecting the pieces of segments that lie far beyond one side of `extent`, the
    /// rectangle that clips the output. A piece stops when its projected midpoint lies beyond
    /// one side by more than twice the distance to its farther projected end, plus the
    /// precision. The piece must also span less than 30°, project its midpoint near the middle
    /// of its chord, and stay farther than its own length from the poles and from the antipode
    /// of the projection's center, where projections tear or bend sharply. A projection bends
    /// such a piece too little to bring it back to the rectangle, so the clipped output is the
    /// same. A randomized test checks this for each built-in projection. A custom projection
    /// that folds short pieces back could lose the parts that come back.
    pub fn with_cull(mut self, extent: Option<[[f64; 2]; 2]>) -> Self {
        self.cull = extent;
        self
    }

    /// Whether a projected point lies beyond one side of the cull rectangle by more than
    /// `margin`.
    fn beyond(&self, (x, y): (f64, f64), margin: f64) -> bool {
        let Some([[x0, y0], [x1, y1]]) = self.cull else {
            return false;
        };
        // Comparisons with NaN fail, so a point that doesn't project keeps bisecting.
        x < x0 - margin || x > x1 + margin || y < y0 - margin || y > y1 + margin
    }

    #[allow(clippy::too_many_arguments)]
    fn resample_line_to(
        &mut self,
        x0: f64,
        y0: f64,
        lambda0: f64,
        a0: f64,
        b0: f64,
        c0: f64,
        x1: f64,
        y1: f64,
        lambda1: f64,
        a1: f64,
        b1: f64,
        c1: f64,
        depth: u32,
    ) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let d2 = dx * dx + dy * dy;
        if d2 > 4.0 * self.delta2 && depth > 0 {
            let depth = depth - 1;
            let mut a = a0 + a1;
            let mut b = b0 + b1;
            let mut c = c0 + c1;
            let m = (a * a + b * b + c * c).sqrt();
            c /= m;
            let phi2 = asin(c);
            let lambda2 = if (c.abs() - 1.0).abs() < EPSILON || (lambda0 - lambda1).abs() < EPSILON
            {
                (lambda0 + lambda1) / 2.0
            } else {
                b.atan2(a)
            };
            let (x2, y2) = (self.project)(lambda2, phi2);
            let dx2 = x2 - x0;
            let dy2 = y2 - y0;
            let dz = dy * dx2 - dx * dy2;
            // The midpoint lies close to an end.
            let skewed = ((dx * dx2 + dy * dy2) / d2 - 0.5).abs() > 0.3;
            let cos_length = a0 * a1 + b0 * b1 + c0 * c1;
            let long = cos_length < cos_min_distance();
            // A short, even piece far beyond one side of the clip rectangle draws nothing.
            let reach = (x0 - x2).hypot(y0 - y2).max((x1 - x2).hypot(y1 - y2));
            if !skewed
                && !long
                && self.beyond((x2, y2), 2.0 * reach + self.delta2.sqrt())
                && clear_of_tears([a0, b0, c0], [a1, b1, c1], cos_length)
            {
                return;
            }
            if dz * dz / d2 > self.delta2 // perpendicular projected distance
                || skewed
                || long
            {
                a /= m;
                b /= m;
                self.resample_line_to(x0, y0, lambda0, a0, b0, c0, x2, y2, lambda2, a, b, c, depth);
                self.sink.point(x2, y2, None);
                self.resample_line_to(x2, y2, lambda2, a, b, c, x1, y1, lambda1, a1, b1, c1, depth);
            }
        }
    }

    fn line_point(&mut self, lambda: f64, phi: f64) {
        let c = cartesian(lambda, phi);
        let (px, py) = (self.project)(lambda, phi);
        let (x0, y0, lambda0, a0, b0, c0) =
            (self.x0, self.y0, self.lambda0, self.a0, self.b0, self.c0);
        self.x0 = px;
        self.y0 = py;
        self.lambda0 = lambda;
        self.a0 = c[0];
        self.b0 = c[1];
        self.c0 = c[2];
        if self.delta2 > 0.0 {
            self.resample_line_to(
                x0,
                y0,
                lambda0,
                a0,
                b0,
                c0,
                self.x0,
                self.y0,
                self.lambda0,
                self.a0,
                self.b0,
                self.c0,
                MAX_DEPTH,
            );
        }
        self.sink.point(self.x0, self.y0, None);
    }
}

impl<F, S> GeoSink for Resample<F, S>
where
    F: Fn(f64, f64) -> (f64, f64),
    S: GeoSink,
{
    fn point(&mut self, lambda: f64, phi: f64, m: Option<f64>) {
        if self.in_line {
            if self.ring_first_point {
                self.ring_first_point = false;
                self.line_point(lambda, phi);
                self.lambda00 = self.lambda0;
                self.x00 = self.x0;
                self.y00 = self.y0;
                self.a00 = self.a0;
                self.b00 = self.b0;
                self.c00 = self.c0;
            } else {
                self.line_point(lambda, phi);
            }
        } else {
            let (x, y) = (self.project)(lambda, phi);
            self.sink.point(x, y, m);
        }
    }

    fn line_start(&mut self) {
        self.x0 = f64::NAN;
        self.lambda0 = f64::NAN;
        self.y0 = f64::NAN;
        self.a0 = f64::NAN;
        self.b0 = f64::NAN;
        self.c0 = f64::NAN;
        self.in_line = true;
        self.ring_first_point = self.in_polygon;
        self.sink.line_start();
    }

    fn line_end(&mut self) {
        if self.in_polygon && self.delta2 > 0.0 && !self.ring_first_point {
            // Close the ring by resampling back to the first point.
            let (x0, y0, lambda0, a0, b0, c0) =
                (self.x0, self.y0, self.lambda0, self.a0, self.b0, self.c0);
            let (x00, y00, lambda00, a00, b00, c00) = (
                self.x00,
                self.y00,
                self.lambda00,
                self.a00,
                self.b00,
                self.c00,
            );
            self.resample_line_to(
                x0, y0, lambda0, a0, b0, c0, x00, y00, lambda00, a00, b00, c00, MAX_DEPTH,
            );
        }
        self.in_line = false;
        self.sink.line_end();
    }

    fn polygon_start(&mut self) {
        self.sink.polygon_start();
        self.in_polygon = true;
    }

    fn polygon_end(&mut self) {
        self.sink.polygon_end();
        self.in_polygon = false;
    }

    fn sphere(&mut self) {
        self.sink.sphere()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::RecordingSink;

    /// With precision 0 (delta2 = 0), vertices pass through unchanged.
    #[test]
    fn no_resampling_at_zero_precision() {
        let mut sink = RecordingSink::default();
        {
            let mut rs = Resample::new(|l, p| (l, p), 0.0, &mut sink);
            rs.line_start();
            rs.point(0.0, 0.0, None);
            rs.point(1.0, 1.0, None);
            rs.line_end();
        }
        assert_eq!(sink.polylines()[0].len(), 2);
    }

    /// A long segment under a curved projection gains interior points.
    #[test]
    fn resamples_curved_segment() {
        // Simulate a projection with curvature: equirectangular scaled up so
        // the great-circle path deviates from the straight line.
        let scale = 200.0;
        let mut sink = RecordingSink::default();
        {
            let mut rs = Resample::new(move |l, p| (l * scale, -p * scale), 0.5, &mut sink);
            rs.line_start();
            rs.point(-1.5, 0.6, None);
            rs.point(1.5, 0.6, None);
            rs.line_end();
        }
        let line = &sink.polylines()[0];
        assert!(
            line.len() > 2,
            "expected interior resampled points, got {}",
            line.len()
        );
    }

    /// Counts the points that pass through it.
    struct Counting<S> {
        points: usize,
        sink: S,
    }

    impl<S: GeoSink> GeoSink for Counting<S> {
        fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
            self.points += 1;
            self.sink.point(x, y, m);
        }
        fn line_start(&mut self) {
            self.sink.line_start();
        }
        fn line_end(&mut self) {
            self.sink.line_end();
        }
        fn polygon_start(&mut self) {
            self.sink.polygon_start();
        }
        fn polygon_end(&mut self) {
            self.sink.polygon_end();
        }
    }

    /// At a deep zoom, culling the pieces beyond the clip rectangle keeps the rectangle clip's
    /// output the same, for lines and rings through the rectangle and beyond it, and bisects
    /// far fewer points.
    #[test]
    fn culling_keeps_the_clipped_output() {
        use crate::clip::rectangle::ClipRectangle;
        // An orthographic view of the front of the sphere, a million units to a radius.
        let scale = 1e6;
        let project =
            move |lambda: f64, phi: f64| (scale * phi.cos() * lambda.sin(), -scale * phi.sin());
        let extent = [[-300.0, -200.0], [300.0, 200.0]];
        let degrees = |from: i32, to: i32| -> Vec<f64> {
            (from..=to)
                .step_by(2)
                .map(|d| f64::from(d) * RADIANS)
                .collect()
        };
        let line = |points: Vec<(f64, f64)>| (false, points);
        let ring = |points: Vec<(f64, f64)>| (true, points);
        let around = |west: i32, east: i32, south: i32, north: i32| {
            let mut points: Vec<(f64, f64)> = degrees(west, east)
                .into_iter()
                .map(|l| (l, f64::from(south) * RADIANS))
                .collect();
            points.extend(
                degrees(south, north)
                    .into_iter()
                    .map(|p| (f64::from(east) * RADIANS, p)),
            );
            points.extend(
                degrees(west, east)
                    .into_iter()
                    .rev()
                    .map(|l| (l, f64::from(north) * RADIANS)),
            );
            points.extend(
                degrees(south, north)
                    .into_iter()
                    .rev()
                    .map(|p| (f64::from(west) * RADIANS, p)),
            );
            points
        };
        let shapes = [
            // The equator, through the rectangle.
            line(degrees(-60, 60).into_iter().map(|l| (l, 0.0)).collect()),
            // A meridian just east of the middle, through the rectangle.
            line(degrees(-60, 60).into_iter().map(|p| (1e-4, p)).collect()),
            // A parallel far beyond the rectangle.
            line(
                degrees(-60, 60)
                    .into_iter()
                    .map(|l| (l, 30.0 * RADIANS))
                    .collect(),
            ),
            // A ring around the rectangle, and a ring beyond it.
            ring(around(-10, 10, -10, 10)),
            ring(around(20, 40, -10, 10)),
        ];
        // Whether the clip keeps some of each shape.
        let shown = [true, true, false, true, false];
        let mut points = [0, 0];
        for ((polygon, shape), shown) in shapes.iter().zip(shown) {
            let outputs: Vec<_> = [None, Some(extent)]
                .into_iter()
                .enumerate()
                .map(|(culled, cull)| {
                    let mut recorded = RecordingSink::default();
                    let mut counted = Counting {
                        points: 0,
                        sink: ClipRectangle::new(extent, &mut recorded),
                    };
                    {
                        let mut resample =
                            Resample::new(project, 0.5, &mut counted).with_cull(cull);
                        if *polygon {
                            resample.polygon_start();
                        }
                        resample.line_start();
                        for &(lambda, phi) in shape {
                            resample.point(lambda, phi, None);
                        }
                        resample.line_end();
                        if *polygon {
                            resample.polygon_end();
                        }
                    }
                    points[culled] += counted.points;
                    recorded.events
                })
                .collect();
            assert_eq!(outputs[0], outputs[1], "polygon: {polygon}");
            assert_eq!(!outputs[0].is_empty(), shown, "polygon: {polygon}");
        }
        // The pieces beyond the rectangle stop bisecting.
        assert!(points[1] * 5 < points[0], "{points:?}");
    }

    /// A small deterministic generator for the randomized test: xorshift64*.
    struct Rng(u64);

    impl Rng {
        /// Uniform in `[lo, hi)`.
        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let unit =
                (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
            lo + (hi - lo) * unit
        }
    }

    /// A segment's resampled line after the rectangle clip, with or without the cull.
    fn clipped(
        project: impl Fn(f64, f64) -> (f64, f64),
        from: (f64, f64),
        to: (f64, f64),
        extent: [[f64; 2]; 2],
        cull: bool,
    ) -> Vec<crate::stream::StreamEvent> {
        use crate::clip::rectangle::ClipRectangle;
        let mut recorded = RecordingSink::default();
        {
            let mut clip = ClipRectangle::new(extent, &mut recorded);
            let mut resample =
                Resample::new(project, 0.5, &mut clip).with_cull(cull.then_some(extent));
            resample.line_start();
            resample.point(from.0, from.1, None);
            resample.point(to.0, to.1, None);
            resample.line_end();
        }
        recorded.events
    }

    /// A line that passes within 4° of a south pole, which Aitoff makes a point, bends there
    /// back into the rectangle, so it keeps bisecting.
    #[test]
    fn a_line_past_a_point_pole_keeps_bisecting() {
        use crate::projections::{Aitoff, ProjectionImpl};
        let project = |lambda: f64, phi: f64| {
            let (x, y) = Aitoff.project(lambda, phi);
            (6284.0 * x, -6284.0 * y)
        };
        let from = (146.8425 * RADIANS, -86.0754 * RADIANS);
        let to = (58.3945 * RADIANS, -63.4837 * RADIANS);
        let extent = [[243.32, 9337.0], [1243.32, 9937.0]];
        let unculled = clipped(project, from, to, extent, false);
        assert!(!unculled.is_empty());
        assert_eq!(clipped(project, from, to, extent, true), unculled);
    }

    /// For every built-in projection, at scales from 10² to 10⁷, segments anywhere and near the
    /// poles and the antipode clip the same with the cull as without it, in rectangles near
    /// their curves.
    #[test]
    fn culling_keeps_the_clipped_output_of_every_projection() {
        use crate::math::{HALF_PI, PI};
        use crate::projections::*;
        use crate::SphericalClip;
        let projections: Vec<(&str, Box<dyn ProjectionImpl>)> = vec![
            ("aitoff", Box::new(Aitoff)),
            ("azimuthal_equal_area", Box::new(AzimuthalEqualArea)),
            ("azimuthal_equidistant", Box::new(AzimuthalEquidistant)),
            (
                "conic_conformal",
                Box::new(ConicConformal::new([35.0, 65.0])),
            ),
            (
                "conic_equal_area",
                Box::new(ConicEqualArea::new([29.5, 45.5])),
            ),
            (
                "conic_equidistant",
                Box::new(ConicEquidistant::new([30.0, 60.0])),
            ),
            (
                "cylindrical_equal_area",
                Box::new(CylindricalEqualArea::new(38.58)),
            ),
            ("eckert4", Box::new(Eckert4)),
            ("equal_earth", Box::new(EqualEarth)),
            ("equirectangular", Box::new(Equirectangular)),
            ("gnomonic", Box::new(Gnomonic)),
            ("hammer", Box::new(Hammer::default())),
            ("mercator", Box::new(Mercator)),
            ("miller", Box::new(Miller)),
            ("mollweide", Box::new(Mollweide)),
            ("natural_earth1", Box::new(NaturalEarth1)),
            ("orthographic", Box::new(Orthographic)),
            ("robinson", Box::new(Robinson)),
            ("sinusoidal", Box::new(Sinusoidal)),
            ("stereographic", Box::new(Stereographic)),
            ("transverse_mercator", Box::new(TransverseMercator)),
            ("winkel_tripel", Box::new(WinkelTripel)),
        ];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for (name, projection) in &projections {
            // Within a circle clip's cap, as the clip ahead of the resampler leaves them.
            let cap = match projection.clip() {
                SphericalClip::Circle { angle } => ((angle - 1.0) * RADIANS).cos(),
                _ => -1.0,
            };
            let mut trials = 0;
            while trials < 300 {
                // One end anywhere, or within 15° of a pole or the antipode of the center.
                let near = rng.range(0.0, 15.0) * RADIANS;
                let around = rng.range(-PI, PI);
                let from = match (rng.range(0.0, 4.0)) as u32 {
                    0 => {
                        let z: f64 = rng.range(-1.0, 1.0);
                        let r = (1.0 - z * z).sqrt();
                        [r * around.cos(), r * around.sin(), z]
                    }
                    1 => cartesian(around, HALF_PI - near),
                    2 => cartesian(around, near - HALF_PI),
                    _ => [
                        -near.cos(),
                        near.sin() * around.cos(),
                        near.sin() * around.sin(),
                    ],
                };
                // The other end, up to 29° away in any direction.
                let length = 10f64.powf(rng.range(-1.3, 1.46)) * RADIANS;
                let east = cartesian_cross(&[0.0, 0.0, 1.0], &from);
                let east_norm = cartesian_dot(&east, &east).sqrt();
                if east_norm < 1e-9 {
                    continue;
                }
                let east = east.map(|v| v / east_norm);
                let north = cartesian_cross(&from, &east);
                let heading = rng.range(-PI, PI);
                let (sin_h, cos_h) = heading.sin_cos();
                let (sin_l, cos_l) = length.sin_cos();
                let to: [f64; 3] = std::array::from_fn(|i| {
                    from[i] * cos_l + (east[i] * cos_h + north[i] * sin_h) * sin_l
                });
                let spherical = |p: [f64; 3]| (p[1].atan2(p[0]), asin(p[2]));
                // Points along the arc, which must stay within the clip and project.
                let samples: Vec<[f64; 3]> = (0..=64)
                    .map(|i| {
                        let t = length * f64::from(i) / 64.0;
                        let (s, c) = t.sin_cos();
                        std::array::from_fn(|k| {
                            from[k] * c + (east[k] * cos_h + north[k] * sin_h) * s
                        })
                    })
                    .collect();
                let lambdas: Vec<f64> = samples.iter().map(|p| spherical(*p).0).collect();
                let crosses = lambdas.windows(2).any(|w| (w[1] - w[0]).abs() > PI);
                let outside = samples.iter().any(|p| p[0] < cap);
                let unit: Vec<(f64, f64)> = samples
                    .iter()
                    .map(|p| {
                        let (lambda, phi) = spherical(*p);
                        projection.project(lambda, phi)
                    })
                    .collect();
                let wild = unit.iter().any(|&(x, y)| {
                    !(x.is_finite() && y.is_finite()) || x.abs().max(y.abs()) > 20.0
                });
                if crosses || outside || wild {
                    continue;
                }
                trials += 1;
                let scale = 10f64.powf(rng.range(2.0, 7.0));
                let project = |lambda: f64, phi: f64| {
                    let (x, y) = projection.project(lambda, phi);
                    (scale * x, -scale * y)
                };
                // A rectangle near a point of the curve, up to its projected size away.
                let (px, py) = unit[(rng.range(0.0, 65.0)) as usize];
                let size = unit
                    .iter()
                    .map(|&(x, y)| (x - unit[0].0).hypot(y - unit[0].1))
                    .fold(0.0, f64::max)
                    * scale;
                let (cx, cy) = (
                    scale * px + rng.range(-1.0, 1.0) * size,
                    -scale * py + rng.range(-1.0, 1.0) * size,
                );
                let (w, h) = (rng.range(50.0, 2000.0), rng.range(50.0, 2000.0));
                let extent = [[cx - w / 2.0, cy - h / 2.0], [cx + w / 2.0, cy + h / 2.0]];
                let (from, to) = (spherical(from), spherical(to));
                assert_eq!(
                    clipped(project, from, to, extent, true),
                    clipped(project, from, to, extent, false),
                    "{name}: {from:?} to {to:?} at scale {scale}, extent {extent:?}"
                );
            }
        }
    }
}
