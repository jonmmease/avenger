//! Adaptive great-circle resampling.
//!
//! Ported from d3-geo `src/projection/resample.js` (ISC),
//! https://github.com/d3/d3-geo. Recursively bisects each segment along
//! the great circle, comparing the projected spherical midpoint against
//! the linear midpoint, until the perpendicular deviation is below
//! `delta2` (squared pixel tolerance in projected output units).

use crate::math::{asin, cartesian, EPSILON, RADIANS};
use crate::stream::GeoStream;

const MAX_DEPTH: u32 = 16;

fn cos_min_distance() -> f64 {
    (30.0 * RADIANS).cos() // cos(minimum angular distance)
}

/// The resample stage: consumes rotated spherical radians, emits projected
/// planar points. `project` is the full raw+affine transform.
pub struct Resample<F, S> {
    project: F,
    delta2: f64,
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
    S: GeoStream,
{
    pub fn new(project: F, delta2: f64, sink: S) -> Self {
        Resample {
            project,
            delta2,
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
            if dz * dz / d2 > self.delta2 // perpendicular projected distance
                || ((dx * dx2 + dy * dy2) / d2 - 0.5).abs() > 0.3 // midpoint close to an end
                || a0 * a1 + b0 * b1 + c0 * c1 < cos_min_distance()
            // angular distance
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

impl<F, S> GeoStream for Resample<F, S>
where
    F: Fn(f64, f64) -> (f64, f64),
    S: GeoStream,
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
}
