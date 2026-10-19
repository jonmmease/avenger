//! Cylindrical equal-area, true to scale along a standard parallel.
//!
//! Ported from d3-geo-projection `src/cylindricalEqualArea.js` (ISC),
//! <https://github.com/d3/d3-geo-projection>

use super::ProjectionImpl;
use crate::error::AvengerGeoError;
use crate::math::{asin, RADIANS};
use serde::{Deserialize, Serialize};

/// Cylindrical equal-area with a standard parallel in degrees: 0 for Lambert, 30 for Behrmann,
/// 45 for Gall–Peters, and 38.58 by d3's default, which fits the world in a 2:1 frame.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CylindricalEqualArea {
    parallel: f64,
}

impl Default for CylindricalEqualArea {
    fn default() -> Self {
        CylindricalEqualArea { parallel: 38.58 }
    }
}

impl CylindricalEqualArea {
    pub fn new(parallel: f64) -> Self {
        CylindricalEqualArea { parallel }
    }

    pub fn parallel(&self) -> f64 {
        self.parallel
    }

    fn cos_phi0(&self) -> f64 {
        (self.parallel * RADIANS).cos()
    }
}

#[typetag::serde(name = "cylindrical_equal_area")]
impl ProjectionImpl for CylindricalEqualArea {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let cos_phi0 = self.cos_phi0();
        (lambda * cos_phi0, phi.sin() / cos_phi0)
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let cos_phi0 = self.cos_phi0();
        Some((x / cos_phi0, asin(y * cos_phi0)))
    }

    fn validate(&self) -> Result<(), AvengerGeoError> {
        if self.parallel.abs() < 90.0 {
            Ok(())
        } else {
            Err(AvengerGeoError::InvalidConfig(format!(
                "a standard parallel must be a latitude strictly between -90 and 90 degrees, not {}",
                self.parallel
            )))
        }
    }
}
