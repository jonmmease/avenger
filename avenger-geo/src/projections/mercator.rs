//! Ported from d3-geo `src/projection/mercator.js` (ISC),
//! <https://github.com/d3/d3-geo>

use super::ProjectionImpl;
use crate::math::{HALF_PI, PI};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Mercator;

#[typetag::serde(name = "mercator")]
impl ProjectionImpl for Mercator {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        (lambda, ((HALF_PI + phi) / 2.0).tan().ln())
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        Some((x, 2.0 * y.exp().atan() - HALF_PI))
    }

    /// The world square (d3 mercator.js `reclip()`): the poles project to
    /// infinity.
    fn bounds(&self) -> Option<[[f64; 2]; 2]> {
        Some([[-PI, -PI], [PI, PI]])
    }
}
