//! Hammer: an equal-area projection of the azimuthal equal-area hemisphere, stretched.
//!
//! Ported from d3-geo-projection `src/hammer.js` (ISC),
//! <https://github.com/d3/d3-geo-projection>

use super::ProjectionImpl;
use crate::error::AvengerGeoError;
use crate::math::asin;
use serde::{Deserialize, Serialize};

/// Hammer with a coefficient, which d3 defaults to 2: the classic Hammer–Aitoff ellipse.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Hammer {
    coefficient: f64,
}

impl Default for Hammer {
    fn default() -> Self {
        Hammer { coefficient: 2.0 }
    }
}

impl Hammer {
    /// Hammer with `coefficient`, which must be finite and at least 1, where Hammer is azimuthal
    /// equal-area. Below 1, the map would fold over itself past the antimeridian.
    pub fn new(coefficient: f64) -> Self {
        Hammer { coefficient }
    }

    pub fn coefficient(&self) -> f64 {
        self.coefficient
    }
}

/// d3 `azimuthalEqualAreaRaw`.
fn azimuthal_equal_area(lambda: f64, phi: f64) -> (f64, f64) {
    let (cx, cy) = (lambda.cos(), phi.cos());
    let k = (2.0 / (1.0 + cx * cy)).sqrt();
    if k == f64::INFINITY {
        return (2.0, 0.0);
    }
    (k * cy * lambda.sin(), k * phi.sin())
}

#[typetag::serde(name = "hammer")]
impl ProjectionImpl for Hammer {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let b = self.coefficient;
        let (x, y) = azimuthal_equal_area(lambda / b, phi);
        (x * b, y)
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let b = self.coefficient;
        // d3 `azimuthalInvert` of azimuthal equal-area.
        let x = x / b;
        let z = (x * x + y * y).sqrt();
        let c = 2.0 * asin(z / 2.0);
        let (sc, cc) = (c.sin(), c.cos());
        let phi = if z == 0.0 { 0.0 } else { asin(y * sc / z) };
        Some(((x * sc).atan2(z * cc) * b, phi))
    }

    fn validate(&self) -> Result<(), AvengerGeoError> {
        if self.coefficient.is_finite() && self.coefficient >= 1.0 {
            Ok(())
        } else {
            Err(AvengerGeoError::InvalidConfig(format!(
                "a Hammer coefficient must be finite and at least 1, not {}",
                self.coefficient
            )))
        }
    }
}
