//! Miller: Mercator with its latitudes compressed, so that the poles are finite.
//!
//! Ported from d3-geo-projection `src/miller.js` (ISC),
//! <https://github.com/d3/d3-geo-projection>

use super::ProjectionImpl;
use crate::math::{PI, QUARTER_PI};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Miller;

#[typetag::serde(name = "miller")]
impl ProjectionImpl for Miller {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        (lambda, 1.25 * (QUARTER_PI + 0.4 * phi).tan().ln())
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        Some((x, 2.5 * (0.8 * y).exp().atan() - 0.625 * PI))
    }
}
