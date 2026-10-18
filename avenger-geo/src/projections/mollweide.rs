//! Mollweide: an equal-area ellipse.
//!
//! Ported from d3-geo-projection `src/mollweide.js` (ISC),
//! <https://github.com/d3/d3-geo-projection>

use super::ProjectionImpl;
use crate::math::{asin, EPSILON, HALF_PI, PI};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Mollweide;

const CX: f64 = std::f64::consts::SQRT_2 / HALF_PI;
const CY: f64 = std::f64::consts::SQRT_2;
const CP: f64 = PI;

/// d3-geo-projection `mollweideBromleyTheta`: Newton's solution of θ + sin θ = cp sin φ, halved.
fn theta(cp: f64, mut phi: f64) -> f64 {
    let cp_sin_phi = cp * phi.sin();
    let mut i = 30;
    loop {
        let delta = (phi + phi.sin() - cp_sin_phi) / (1.0 + phi.cos());
        phi -= delta;
        i -= 1;
        if !(delta.abs() > EPSILON && i > 0) {
            break;
        }
    }
    phi / 2.0
}

#[typetag::serde(name = "mollweide")]
impl ProjectionImpl for Mollweide {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let phi = theta(CP, phi);
        (CX * lambda * phi.cos(), CY * phi.sin())
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let y = asin(y / CY);
        Some((x / (CX * y.cos()), asin((2.0 * y + (2.0 * y).sin()) / CP)))
    }
}
