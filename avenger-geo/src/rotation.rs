//! Three-axis spherical rotation (λ, φ, γ).
//!
//! Ported from d3-geo `src/rotation.js` and `src/compose.js` (ISC),
//! https://github.com/d3/d3-geo

use crate::math::{asin, PI, TAU};

/// A composed spherical rotation working in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation {
    delta_lambda: f64,
    /// None when there is no φ/γ component.
    phi_gamma: Option<PhiGamma>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PhiGamma {
    cos_delta_phi: f64,
    sin_delta_phi: f64,
    cos_delta_gamma: f64,
    sin_delta_gamma: f64,
}

fn wrap_lambda(lambda: f64) -> f64 {
    if lambda > PI {
        lambda - TAU
    } else if lambda < -PI {
        lambda + TAU
    } else {
        lambda
    }
}

impl Rotation {
    /// Angles in radians (d3 `rotateRadians`).
    pub fn new(delta_lambda: f64, delta_phi: f64, delta_gamma: f64) -> Self {
        let delta_lambda = delta_lambda % TAU;
        let phi_gamma = if delta_phi != 0.0 || delta_gamma != 0.0 {
            Some(PhiGamma {
                cos_delta_phi: delta_phi.cos(),
                sin_delta_phi: delta_phi.sin(),
                cos_delta_gamma: delta_gamma.cos(),
                sin_delta_gamma: delta_gamma.sin(),
            })
        } else {
            None
        };
        Rotation {
            delta_lambda,
            phi_gamma,
        }
    }

    /// Angles in degrees, matching `projection.rotate([λ, φ, γ])`.
    pub fn from_degrees(rotate: [f64; 3]) -> Self {
        Rotation::new(
            rotate[0] * crate::math::RADIANS,
            rotate[1] * crate::math::RADIANS,
            rotate[2] * crate::math::RADIANS,
        )
    }

    pub fn is_identity(&self) -> bool {
        self.delta_lambda == 0.0 && self.phi_gamma.is_none()
    }

    /// Forward rotation of a spherical point (radians).
    pub fn rotate(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let lambda = wrap_lambda(lambda + self.delta_lambda);
        match &self.phi_gamma {
            None => (lambda, phi),
            Some(pg) => pg.forward(lambda, phi),
        }
    }

    /// Inverse rotation of a spherical point (radians).
    pub fn invert(&self, lambda: f64, phi: f64) -> (f64, f64) {
        match &self.phi_gamma {
            None => (wrap_lambda(lambda - self.delta_lambda), phi),
            Some(pg) => {
                let (l, p) = pg.invert(lambda, phi);
                (wrap_lambda(l - self.delta_lambda), p)
            }
        }
    }
}

impl PhiGamma {
    fn forward(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let cos_phi = phi.cos();
        let x = lambda.cos() * cos_phi;
        let y = lambda.sin() * cos_phi;
        let z = phi.sin();
        let k = z * self.cos_delta_phi + x * self.sin_delta_phi;
        (
            (y * self.cos_delta_gamma - k * self.sin_delta_gamma)
                .atan2(x * self.cos_delta_phi - z * self.sin_delta_phi),
            asin(k * self.cos_delta_gamma + y * self.sin_delta_gamma),
        )
    }

    fn invert(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let cos_phi = phi.cos();
        let x = lambda.cos() * cos_phi;
        let y = lambda.sin() * cos_phi;
        let z = phi.sin();
        let k = z * self.cos_delta_gamma - y * self.sin_delta_gamma;
        (
            (y * self.cos_delta_gamma + z * self.sin_delta_gamma)
                .atan2(x * self.cos_delta_phi + k * self.sin_delta_phi),
            asin(k * self.cos_delta_phi - x * self.sin_delta_phi),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::RADIANS;

    #[test]
    fn round_trips() {
        for rotate in [
            [0.0, 0.0, 0.0],
            [96.0, 0.0, 0.0],
            [15.0, -30.0, 0.0],
            [15.0, -30.0, 12.0],
            [-120.0, 45.0, 90.0],
        ] {
            let r = Rotation::from_degrees(rotate);
            for &(lon, lat) in &[(0.0, 0.0), (10.0, 20.0), (-170.0, -80.0), (179.0, 89.0)] {
                let (l, p) = r.rotate(lon * RADIANS, lat * RADIANS);
                let (l2, p2) = r.invert(l, p);
                assert!(
                    (l2 - lon * RADIANS).abs() < 1e-12 && (p2 - lat * RADIANS).abs() < 1e-12,
                    "rotate {rotate:?} point ({lon}, {lat})"
                );
            }
        }
    }

    /// d3: geoRotation([90, 0])([0, 0]) == [90, 0]
    #[test]
    fn rotates_lambda() {
        let r = Rotation::from_degrees([90.0, 0.0, 0.0]);
        let (l, p) = r.rotate(0.0, 0.0);
        assert!((l - 90.0 * RADIANS).abs() < 1e-12 && p.abs() < 1e-12);
    }

    /// d3: geoRotation([0, -45])([0, 0]) == [0, -45]
    #[test]
    fn rotates_phi() {
        let r = Rotation::from_degrees([0.0, -45.0, 0.0]);
        let (l, p) = r.rotate(0.0, 0.0);
        assert!(l.abs() < 1e-12 && (p + 45.0 * RADIANS).abs() < 1e-12);
    }
}
