//! Ported from d3-geo `src/projection/equirectangular.js` (ISC),
//! <https://github.com/d3/d3-geo>

use super::ProjectionImpl;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Equirectangular;

#[typetag::serde(name = "equirectangular")]
impl ProjectionImpl for Equirectangular {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        (lambda, phi)
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        Some((x, y))
    }
}
