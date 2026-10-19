//! Sinusoidal: an equal-area projection whose parallels are evenly spaced.
//!
//! Ported from d3-geo-projection `src/sinusoidal.js` (ISC),
//! <https://github.com/d3/d3-geo-projection>

use super::ProjectionImpl;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Sinusoidal;

#[typetag::serde(name = "sinusoidal")]
impl ProjectionImpl for Sinusoidal {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        (lambda * phi.cos(), phi)
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        Some((x / y.cos(), y))
    }
}
