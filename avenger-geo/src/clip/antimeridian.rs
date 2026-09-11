//! Antimeridian cutting.
//!
//! Ported from d3-geo `src/clip/antimeridian.js` (ISC),
//! https://github.com/d3/d3-geo

use super::{BufPoint, ClipPolicy, LineClipper};
use crate::math::{EPSILON, HALF_PI, PI};
use crate::stream::GeoStream;
use std::cmp::Ordering;

pub struct AntimeridianPolicy;

impl ClipPolicy for AntimeridianPolicy {
    type Line = AntimeridianLine;

    fn point_visible(&self, _lambda: f64, _phi: f64) -> bool {
        true
    }

    fn line_clipper(&self) -> AntimeridianLine {
        AntimeridianLine::default()
    }

    fn interpolate(
        &self,
        from: Option<[f64; 2]>,
        to: Option<[f64; 2]>,
        direction: i32,
        sink: &mut dyn GeoStream,
    ) {
        let direction = direction as f64;
        match (from, to) {
            (None, _) | (_, None) => {
                let phi = direction * HALF_PI;
                sink.point(-PI, phi, None);
                sink.point(0.0, phi, None);
                sink.point(PI, phi, None);
                sink.point(PI, 0.0, None);
                sink.point(PI, -phi, None);
                sink.point(0.0, -phi, None);
                sink.point(-PI, -phi, None);
                sink.point(-PI, 0.0, None);
                sink.point(-PI, phi, None);
            }
            (Some(from), Some(to)) => {
                if (from[0] - to[0]).abs() > EPSILON {
                    let lambda = if from[0] < to[0] { PI } else { -PI };
                    let phi = direction * lambda / 2.0;
                    sink.point(-lambda, phi, None);
                    sink.point(0.0, phi, None);
                    sink.point(lambda, phi, None);
                } else {
                    sink.point(to[0], to[1], None);
                }
            }
        }
    }

    fn start_point(&self) -> [f64; 2] {
        [-PI, -HALF_PI]
    }

    fn compare_intersection(&self, a: &BufPoint, b: &BufPoint) -> Ordering {
        fn key(p: &BufPoint) -> f64 {
            if p.x < 0.0 {
                p.y - HALF_PI - EPSILON
            } else {
                HALF_PI - p.y
            }
        }
        key(a).partial_cmp(&key(b)).unwrap_or(Ordering::Equal)
    }
}

/// Antimeridian line-clip state machine (d3 `clipAntimeridianLine`).
pub struct AntimeridianLine {
    lambda0: f64,
    phi0: f64,
    sign0: f64,
    clean: u8,
}

impl Default for AntimeridianLine {
    fn default() -> Self {
        AntimeridianLine {
            lambda0: f64::NAN,
            phi0: f64::NAN,
            sign0: f64::NAN,
            clean: 0,
        }
    }
}

fn intersect_phi(lambda0: f64, phi0: f64, lambda1: f64, phi1: f64) -> f64 {
    let sin_lambda0_lambda1 = (lambda0 - lambda1).sin();
    if sin_lambda0_lambda1.abs() > EPSILON {
        let cos_phi0 = phi0.cos();
        let cos_phi1 = phi1.cos();
        ((phi0.sin() * cos_phi1 * lambda1.sin() - phi1.sin() * cos_phi0 * lambda0.sin())
            / (cos_phi0 * cos_phi1 * sin_lambda0_lambda1))
            .atan()
    } else {
        (phi0 + phi1) / 2.0
    }
}

impl LineClipper for AntimeridianLine {
    fn line_start(&mut self, sink: &mut dyn GeoStream) {
        sink.line_start();
        self.clean = 1;
        self.lambda0 = f64::NAN;
        self.phi0 = f64::NAN;
        self.sign0 = f64::NAN;
    }

    fn point(&mut self, mut lambda1: f64, phi1: f64, _m: Option<f64>, sink: &mut dyn GeoStream) {
        let sign1 = if lambda1 > 0.0 { PI } else { -PI };
        let delta = (lambda1 - self.lambda0).abs();

        if (delta - PI).abs() < EPSILON {
            // Line crosses a pole: insert two points at the pole latitude.
            let phi = if (self.phi0 + phi1) / 2.0 > 0.0 {
                HALF_PI
            } else {
                -HALF_PI
            };
            self.phi0 = phi;
            sink.point(self.lambda0, phi, None);
            sink.point(self.sign0, phi, None);
            sink.line_end();
            sink.line_start();
            sink.point(sign1, phi, None);
            sink.point(lambda1, phi, None);
            self.clean = 0;
        } else if self.sign0 != sign1 && delta >= PI {
            // Line crosses the antimeridian.
            let mut lambda0 = self.lambda0;
            if (lambda0 - self.sign0).abs() < EPSILON {
                lambda0 -= self.sign0 * EPSILON; // handle degeneracies
            }
            if (lambda1 - sign1).abs() < EPSILON {
                lambda1 -= sign1 * EPSILON;
            }
            let phi = intersect_phi(lambda0, self.phi0, lambda1, phi1);
            self.phi0 = phi;
            sink.point(self.sign0, phi, None);
            sink.line_end();
            sink.line_start();
            sink.point(sign1, phi, None);
            self.clean = 0;
        }

        self.lambda0 = lambda1;
        self.phi0 = phi1;
        sink.point(self.lambda0, self.phi0, None);
        self.sign0 = sign1;
    }

    fn line_end(&mut self, sink: &mut dyn GeoStream) {
        sink.line_end();
        self.lambda0 = f64::NAN;
        self.phi0 = f64::NAN;
    }

    fn clean(&self) -> u8 {
        2 - self.clean
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;
    use crate::math::RADIANS;
    use crate::stream::{RecordingSink, StreamEvent};

    #[test]
    fn cuts_crossing_line() {
        let mut sink = RecordingSink::default();
        {
            let mut clip = Clip::new(AntimeridianPolicy, &mut sink);
            clip.line_start();
            clip.point(170.0 * RADIANS, 10.0 * RADIANS, None);
            clip.point(-170.0 * RADIANS, 12.0 * RADIANS, None);
            clip.line_end();
        }
        // Expect two lines (cut at the antimeridian).
        let lines = sink.polylines();
        assert_eq!(lines.len(), 2, "events: {:?}", sink.events);
        // First line ends at +PI, second starts at -PI.
        let first_end = lines[0].last().unwrap();
        let second_start = lines[1].first().unwrap();
        assert!((first_end[0] - PI).abs() < 1e-9);
        assert!((second_start[0] + PI).abs() < 1e-9);
        // Intersection latitudes match.
        assert!((first_end[1] - second_start[1]).abs() < 1e-12);
    }

    #[test]
    fn straight_line_unchanged() {
        let mut sink = RecordingSink::default();
        {
            let mut clip = Clip::new(AntimeridianPolicy, &mut sink);
            clip.line_start();
            clip.point(0.0, 0.0, None);
            clip.point(0.5, 0.5, None);
            clip.line_end();
        }
        assert_eq!(sink.polylines().len(), 1);
        assert_eq!(sink.polylines()[0].len(), 2);
    }

    #[test]
    fn polygon_spanning_antimeridian_is_rejoined() {
        // A small square straddling the antimeridian, wound in the d3
        // spherical convention (clockwise in y-up planar terms) so the small
        // region is the interior.
        let ring = [[175.0, -5.0], [175.0, 5.0], [-175.0, 5.0], [-175.0, -5.0]];
        let mut sink = RecordingSink::default();
        {
            let mut clip = Clip::new(AntimeridianPolicy, &mut sink);
            clip.polygon_start();
            clip.line_start();
            for p in ring {
                clip.point(p[0] * RADIANS, p[1] * RADIANS, None);
            }
            clip.line_end();
            clip.polygon_end();
        }
        // Should produce a polygon with two rings (one on each side).
        let starts = sink
            .events
            .iter()
            .filter(|e| matches!(e, StreamEvent::LineStart))
            .count();
        assert_eq!(starts, 2, "events: {:?}", sink.events);
        assert!(sink
            .events
            .iter()
            .any(|e| matches!(e, StreamEvent::PolygonStart)));
    }

    #[test]
    fn non_crossing_polygon_passes_through() {
        let ring = [[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0]];
        let mut sink = RecordingSink::default();
        {
            let mut clip = Clip::new(AntimeridianPolicy, &mut sink);
            clip.polygon_start();
            clip.line_start();
            for p in ring {
                clip.point(p[0] * RADIANS, p[1] * RADIANS, None);
            }
            clip.line_end();
            clip.polygon_end();
        }
        let lines = sink.polylines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 4, "closing point omitted: {:?}", lines);
    }
}
