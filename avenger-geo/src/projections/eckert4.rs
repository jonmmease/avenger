//! Eckert IV: an equal-area projection with rounded sides and a flat pole line.
//!
//! Ported from d3-geo-projection `src/eckert4.js` (ISC),
//! <https://github.com/d3/d3-geo-projection>

use super::ProjectionImpl;
use crate::math::{asin, EPSILON, HALF_PI, PI};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Eckert4;

#[typetag::serde(name = "eckert4")]
impl ProjectionImpl for Eckert4 {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let k = (2.0 + HALF_PI) * phi.sin();
        let mut phi = phi / 2.0;
        let mut delta = f64::INFINITY;
        let mut i = 0;
        while i < 10 && delta.abs() > EPSILON {
            let cos_phi = phi.cos();
            delta = (phi + phi.sin() * (cos_phi + 2.0) - k) / (2.0 * cos_phi * (1.0 + cos_phi));
            phi -= delta;
            i += 1;
        }
        (
            2.0 / (PI * (4.0 + PI)).sqrt() * lambda * (1.0 + phi.cos()),
            2.0 * (PI / (4.0 + PI)).sqrt() * phi.sin(),
        )
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let a = y * ((4.0 + PI) / PI).sqrt() / 2.0;
        let k = asin(a);
        let c = k.cos();
        Some((
            x / (2.0 / (PI * (4.0 + PI)).sqrt() * (1.0 + c)),
            asin((k + a * (c + 2.0)) / (2.0 + HALF_PI)),
        ))
    }
}
