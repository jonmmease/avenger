//! Transverse Mercator: Mercator turned on its side, so that it follows a meridian.
//!
//! Ported from d3-geo `src/projection/transverseMercator.js` (ISC),
//! <https://github.com/d3/d3-geo>. d3 rolls the sphere by 90° as part of the
//! caller's rotation, which [`ProjectionImpl::rotation`] does here. d3 also
//! swaps a center's coordinates, where this crate turns the center by the same
//! roll, so the two place a center the same way only at the origin.

use super::ProjectionImpl;
use crate::math::{HALF_PI, PI};
use serde::{Deserialize, Serialize};

/// Transverse Mercator, conformal along its central meridian, which `rotate` places.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TransverseMercator;

#[typetag::serde(name = "transverse_mercator")]
impl ProjectionImpl for TransverseMercator {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        (((HALF_PI + phi) / 2.0).tan().ln(), -lambda)
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        Some((-y, 2.0 * x.exp().atan() - HALF_PI))
    }

    /// Mercator's world square, around the poles of the rolled sphere.
    fn bounds(&self) -> Option<[[f64; 2]; 2]> {
        Some([[-PI, -PI], [PI, PI]])
    }

    fn rotation(&self) -> [f64; 3] {
        [0.0, 0.0, 90.0]
    }
}
