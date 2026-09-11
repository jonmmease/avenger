//! Conic equal-area (Albers when paired with the standard parallels).
//!
//! Ported from d3-geo `src/projection/conicEqualArea.js` and
//! `src/projection/cylindricalEqualArea.js` (ISC),
//! <https://github.com/d3/d3-geo>

use super::RawProjection;
use crate::math::{asin, sign, EPSILON, PI};

#[derive(Debug, Clone, Copy)]
pub enum ConicEqualAreaRaw {
    Conic {
        n: f64,
        c: f64,
        r0: f64,
    },
    /// Degenerate case when the parallels are symmetric about the equator.
    Cylindrical {
        cos_phi0: f64,
    },
}

impl ConicEqualAreaRaw {
    /// Parallels in radians.
    pub fn new(y0: f64, y1: f64) -> Self {
        let sy0 = y0.sin();
        let n = (sy0 + y1.sin()) / 2.0;
        if n.abs() < EPSILON {
            return ConicEqualAreaRaw::Cylindrical { cos_phi0: y0.cos() };
        }
        let c = 1.0 + sy0 * (2.0 * n - sy0);
        let r0 = c.sqrt() / n;
        ConicEqualAreaRaw::Conic { n, c, r0 }
    }
}

impl RawProjection for ConicEqualAreaRaw {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        match *self {
            ConicEqualAreaRaw::Conic { n, c, r0 } => {
                let r = (c - 2.0 * n * phi.sin()).max(0.0).sqrt() / n;
                let x = lambda * n;
                (r * x.sin(), r0 - r * x.cos())
            }
            ConicEqualAreaRaw::Cylindrical { cos_phi0 } => {
                (lambda * cos_phi0, phi.sin() / cos_phi0)
            }
        }
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        match *self {
            ConicEqualAreaRaw::Conic { n, c, r0 } => {
                let r0y = r0 - y;
                let mut l = x.atan2(r0y.abs()) * sign(r0y);
                if r0y * n < 0.0 {
                    l -= PI * sign(x) * sign(r0y);
                }
                Some((l / n, asin((c - (x * x + r0y * r0y) * n * n) / (2.0 * n))))
            }
            ConicEqualAreaRaw::Cylindrical { cos_phi0 } => Some((x / cos_phi0, asin(y * cos_phi0))),
        }
    }
}
