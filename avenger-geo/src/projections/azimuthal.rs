//! Azimuthal projections, each clipped by default to a circle around its center.
//!
//! Ported from d3-geo `src/projection/azimuthal.js`, `orthographic.js`,
//! `stereographic.js`, `gnomonic.js`, `azimuthalEqualArea.js`, and
//! `azimuthalEquidistant.js` (ISC), <https://github.com/d3/d3-geo>

use super::ProjectionImpl;
use crate::math::{acos, asin, EPSILON, PI};
use crate::projector::SphericalClip;
use serde::{Deserialize, Serialize};

/// d3 `azimuthalRaw`: the point at `scale(cos λ cos φ)` times its direction from the center.
fn azimuthal(lambda: f64, phi: f64, scale: impl Fn(f64) -> f64) -> (f64, f64) {
    let (cx, cy) = (lambda.cos(), phi.cos());
    let k = scale(cx * cy);
    if k == f64::INFINITY {
        return (2.0, 0.0);
    }
    (k * cy * lambda.sin(), k * phi.sin())
}

/// d3 `azimuthalInvert`: the point at angle `angle(z)` from the center, `z` the radius. A point
/// farther than `disk`, the radius of the projected sphere, is on no place, where d3 clamps it to
/// the edge.
fn azimuthal_invert(x: f64, y: f64, disk: f64, angle: impl Fn(f64) -> f64) -> Option<(f64, f64)> {
    let z = (x * x + y * y).sqrt();
    if z > disk + EPSILON {
        return None;
    }
    let c = angle(z);
    let (sc, cc) = (c.sin(), c.cos());
    let phi = if z == 0.0 { 0.0 } else { asin(y * sc / z) };
    Some(((x * sc).atan2(z * cc), phi))
}

/// Orthographic: the globe as seen from far away, clipped to its near hemisphere.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Orthographic;

#[typetag::serde(name = "orthographic")]
impl ProjectionImpl for Orthographic {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        (phi.cos() * lambda.sin(), phi.sin())
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        azimuthal_invert(x, y, 1.0, asin)
    }

    fn clip(&self) -> SphericalClip {
        SphericalClip::Circle {
            angle: 90.0 + EPSILON,
        }
    }
}

/// Stereographic: conformal, clipped by default to 142° from its center.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Stereographic;

#[typetag::serde(name = "stereographic")]
impl ProjectionImpl for Stereographic {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let cy = phi.cos();
        let k = 1.0 + lambda.cos() * cy;
        (cy * lambda.sin() / k, phi.sin() / k)
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        azimuthal_invert(x, y, f64::INFINITY, |z| 2.0 * z.atan())
    }

    fn clip(&self) -> SphericalClip {
        SphericalClip::Circle { angle: 142.0 }
    }
}

/// Gnomonic: great circles project to straight lines. Clipped by default to 60° from its center,
/// since the horizon projects to infinity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Gnomonic;

#[typetag::serde(name = "gnomonic")]
impl ProjectionImpl for Gnomonic {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let cy = phi.cos();
        let k = lambda.cos() * cy;
        (cy * lambda.sin() / k, phi.sin() / k)
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        azimuthal_invert(x, y, f64::INFINITY, f64::atan)
    }

    fn clip(&self) -> SphericalClip {
        SphericalClip::Circle { angle: 60.0 }
    }
}

/// Lambert azimuthal equal-area, which shows nearly the whole sphere.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AzimuthalEqualArea;

#[typetag::serde(name = "azimuthal_equal_area")]
impl ProjectionImpl for AzimuthalEqualArea {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        azimuthal(lambda, phi, |cxcy| (2.0 / (1.0 + cxcy)).sqrt())
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        azimuthal_invert(x, y, 2.0, |z| 2.0 * asin(z / 2.0))
    }

    fn clip(&self) -> SphericalClip {
        SphericalClip::Circle {
            angle: 180.0 - 1e-3,
        }
    }
}

/// Azimuthal equidistant: distances from the center are true, out to nearly its antipode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AzimuthalEquidistant;

#[typetag::serde(name = "azimuthal_equidistant")]
impl ProjectionImpl for AzimuthalEquidistant {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        azimuthal(lambda, phi, |cxcy| {
            let c = acos(cxcy);
            if c == 0.0 {
                0.0
            } else {
                c / c.sin()
            }
        })
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        azimuthal_invert(x, y, PI, |z| z)
    }

    fn clip(&self) -> SphericalClip {
        SphericalClip::Circle {
            angle: 180.0 - 1e-3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Projection;

    /// A point off the projected sphere's disk is on no place.
    #[test]
    fn inverses_miss_off_the_disk() {
        let disks: [(&dyn ProjectionImpl, f64); 3] = [
            (&Orthographic, 1.0),
            (&AzimuthalEqualArea, 2.0),
            (&AzimuthalEquidistant, PI),
        ];
        for (projection, disk) in disks {
            assert!(
                projection.invert(0.999 * disk, 0.0).is_some(),
                "{projection:?}"
            );
            assert_eq!(projection.invert(0.0, 1.01 * disk), None, "{projection:?}");
        }
        // Stereographic and gnomonic projections cover the plane.
        assert!(Stereographic.invert(1e6, 0.0).is_some());
        assert!(Gnomonic.invert(1e6, 0.0).is_some());
    }

    /// A stereographic projection's center's antipode projects to infinity, so a view can't be
    /// centered on it.
    #[test]
    fn stereographic_cannot_center_on_its_antipode() {
        let projection = Projection::new(Stereographic)
            .with_center(180.0, 0.0)
            .with_view(100.0, [0.0, 0.0]);
        assert!(projection.build().is_err());
    }
}
