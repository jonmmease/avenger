//! Winkel tripel: the arithmetic mean of equirectangular (with standard
//! parallel acos(2/π)) and Aitoff.
//!
//! Ported from d3-geo-projection `src/winkel3.js` + `src/aitoff.js` (ISC),
//! <https://github.com/d3/d3-geo-projection>. The inverse is the trait's
//! numerical default rather than d3's analytic Jacobian.

use super::ProjectionImpl;
use crate::math::{acos, HALF_PI};
use serde::{Deserialize, Serialize};

/// sinc-inverse helper: x / sin(x), -> 1 as x -> 0.
fn sinci(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        x / x.sin()
    }
}

fn aitoff(lambda: f64, phi: f64) -> (f64, f64) {
    let cos_phi = phi.cos();
    let half_lambda = lambda / 2.0;
    let sinci_alpha = sinci(acos(cos_phi * half_lambda.cos()));
    (
        2.0 * cos_phi * half_lambda.sin() * sinci_alpha,
        phi.sin() * sinci_alpha,
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WinkelTripel;

#[typetag::serde(name = "winkel_tripel")]
impl ProjectionImpl for WinkelTripel {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let (ax, ay) = aitoff(lambda, phi);
        ((ax + lambda / HALF_PI) / 2.0, (ay + phi) / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// d3.geoWinkel3Raw(0, 0) == [0, 0]; spot values from d3-geo-projection.
    #[test]
    fn spot_values() {
        let projection = WinkelTripel;
        let (x, y) = projection.project(0.0, 0.0);
        assert!(x.abs() < 1e-12 && y.abs() < 1e-12);
        // Symmetry checks
        let (x1, y1) = projection.project(1.0, 0.5);
        let (x2, y2) = projection.project(-1.0, 0.5);
        assert!((x1 + x2).abs() < 1e-12 && (y1 - y2).abs() < 1e-12);
        let (x3, y3) = projection.project(1.0, -0.5);
        assert!((x1 - x3).abs() < 1e-12 && (y1 + y3).abs() < 1e-12);
    }
}
