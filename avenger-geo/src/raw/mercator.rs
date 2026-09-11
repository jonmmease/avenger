//! Ported from d3-geo `src/projection/mercator.js` (ISC),
//! https://github.com/d3/d3-geo

use super::RawProjection;
use crate::math::{HALF_PI, QUARTER_PI};

#[derive(Debug, Clone, Copy)]
pub struct MercatorRaw;

impl RawProjection for MercatorRaw {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        (lambda, ((HALF_PI + phi) / 2.0).tan().ln())
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        Some((x, 2.0 * y.exp().atan() - HALF_PI))
    }
}

/// The unit-scale planar half-extent of the mercator world square
/// (x in [-π, π], y in [-π, π] after the ±85.051° latitude clamp).
pub const MERCATOR_UNIT_LIMIT: f64 = std::f64::consts::PI;

/// Latitude (degrees) beyond which web-mercator output exceeds the world
/// square.
pub fn max_latitude_deg() -> f64 {
    (2.0 * std::f64::consts::E.powf(std::f64::consts::PI).atan() - HALF_PI) * crate::math::DEGREES
}

/// Forward-project latitude only (radians), the separable y component.
pub fn mercator_y(phi: f64) -> f64 {
    ((QUARTER_PI) + phi / 2.0).tan().ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_latitude_matches_web_mercator_constant() {
        assert!((max_latitude_deg() - 85.051_128_779_806_6).abs() < 1e-9);
    }

    #[test]
    fn mercator_y_matches_project() {
        let raw = MercatorRaw;
        for phi in [-1.2, -0.3, 0.0, 0.7, 1.4] {
            assert!((raw.project(0.0, phi).1 - mercator_y(phi)).abs() < 1e-14);
        }
    }
}
