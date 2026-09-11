//! Conic conformal (Lambert).
//!
//! Ported from d3-geo `src/projection/conicConformal.js` (ISC),
//! https://github.com/d3/d3-geo

use super::{MercatorRaw, RawProjection};
use crate::math::{sign, tany, EPSILON, HALF_PI, PI};

#[derive(Debug, Clone, Copy)]
pub enum ConicConformalRaw {
    Conic {
        n: f64,
        f: f64,
    },
    /// Degenerate case (n == 0): mercator.
    Mercator,
}

impl ConicConformalRaw {
    /// Parallels in radians.
    pub fn new(y0: f64, y1: f64) -> Self {
        let cy0 = y0.cos();
        let n = if y0 == y1 {
            y0.sin()
        } else {
            (cy0 / y1.cos()).ln() / (tany(y1) / tany(y0)).ln()
        };
        if n == 0.0 || !n.is_finite() {
            return ConicConformalRaw::Mercator;
        }
        let f = cy0 * tany(y0).powf(n) / n;
        ConicConformalRaw::Conic { n, f }
    }
}

impl RawProjection for ConicConformalRaw {
    fn project(&self, lambda: f64, mut phi: f64) -> (f64, f64) {
        match *self {
            ConicConformalRaw::Conic { n, f } => {
                if f > 0.0 {
                    if phi < -HALF_PI + EPSILON {
                        phi = -HALF_PI + EPSILON;
                    }
                } else if phi > HALF_PI - EPSILON {
                    phi = HALF_PI - EPSILON;
                }
                let r = f / tany(phi).powf(n);
                (r * (n * lambda).sin(), f - r * (n * lambda).cos())
            }
            ConicConformalRaw::Mercator => MercatorRaw.project(lambda, phi),
        }
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        match *self {
            ConicConformalRaw::Conic { n, f } => {
                let fy = f - y;
                let r = sign(n) * (x * x + fy * fy).sqrt();
                let mut l = x.atan2(fy.abs()) * sign(fy);
                if fy * n < 0.0 {
                    l -= PI * sign(x) * sign(fy);
                }
                Some((l / n, 2.0 * (f / r).powf(1.0 / n).atan() - HALF_PI))
            }
            ConicConformalRaw::Mercator => MercatorRaw.invert(x, y),
        }
    }
}
