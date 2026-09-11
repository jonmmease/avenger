//! Raw projections: pure `(λ, φ) -> (x, y)` functions at unit scale
//! (radians in, dimensionless planar units out), plus inverses.
//!
//! The pipeline (rotation, clipping, resampling, scale/translate) is layered
//! around these by [`crate::projector`]; a raw projection contains no view
//! state.

mod conic_conformal;
mod conic_equal_area;
mod equal_earth;
mod equirectangular;
mod identity;
mod mercator;
mod natural_earth1;
mod winkel_tripel;

pub use conic_conformal::ConicConformalRaw;
pub use conic_equal_area::ConicEqualAreaRaw;
pub use equal_earth::EqualEarthRaw;
pub use equirectangular::EquirectangularRaw;
pub use identity::IdentityRaw;
pub use mercator::{max_latitude_deg, mercator_y, MercatorRaw, MERCATOR_UNIT_LIMIT};
pub use natural_earth1::NaturalEarth1Raw;
pub use winkel_tripel::WinkelTripelRaw;

use crate::math::{DEGREES, EPSILON2, RADIANS};
use serde::{Deserialize, Serialize};

/// A raw projection: forward maps spherical radians to unit-scale planar
/// coordinates; `invert` is the inverse where defined.
pub trait RawProjection: Send + Sync {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64);
    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)>;
}

/// Projection catalog. Spherical inputs use degrees at the pipeline boundary.
/// Some projections have singularities, such as the Mercator poles.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectionKind {
    Equirectangular,
    Mercator,
    #[default]
    EqualEarth,
    NaturalEarth1,
    WinkelTripel,
    ConicEqualArea {
        parallels: (f64, f64),
    },
    ConicConformal {
        parallels: (f64, f64),
    },
    /// Planar passthrough for pre-projected data. `reflect_y` flips the
    /// y axis (GIS y-up data on a y-down screen).
    Identity {
        reflect_y: bool,
    },
}

impl ProjectionKind {
    /// Instantiate the raw projection math.
    pub fn raw(&self) -> Box<dyn RawProjection> {
        match self {
            ProjectionKind::Equirectangular => Box::new(EquirectangularRaw),
            ProjectionKind::Mercator => Box::new(MercatorRaw),
            ProjectionKind::EqualEarth => Box::new(EqualEarthRaw),
            ProjectionKind::NaturalEarth1 => Box::new(NaturalEarth1Raw),
            ProjectionKind::WinkelTripel => Box::new(WinkelTripelRaw),
            ProjectionKind::ConicEqualArea { parallels } => Box::new(ConicEqualAreaRaw::new(
                parallels.0 * RADIANS,
                parallels.1 * RADIANS,
            )),
            ProjectionKind::ConicConformal { parallels } => Box::new(ConicConformalRaw::new(
                parallels.0 * RADIANS,
                parallels.1 * RADIANS,
            )),
            ProjectionKind::Identity { reflect_y } => Box::new(IdentityRaw {
                reflect_y: *reflect_y,
            }),
        }
    }

    /// Conic equal-area with the standard CONUS Albers parallels
    /// (29.5°, 45.5°). Combine with `rotate = [96, 0]` for the classic
    /// continental-US aspect (d3.geoAlbers).
    pub fn albers() -> Self {
        ProjectionKind::ConicEqualArea {
            parallels: (29.5, 45.5),
        }
    }

    /// True when the raw projection consumes planar (pre-projected) input
    /// rather than spherical degrees.
    pub fn is_identity(&self) -> bool {
        matches!(self, ProjectionKind::Identity { .. })
    }
}

/// Generic 2D Newton inversion with a finite-difference Jacobian.
///
/// Used by raws without a closed-form inverse (Winkel tripel, blends).
/// Follows the shape of d3-geo-projection's `invert` fallback: seed at the
/// equirectangular estimate and iterate.
pub fn invert_newton<F>(project: F, x: f64, y: f64) -> Option<(f64, f64)>
where
    F: Fn(f64, f64) -> (f64, f64),
{
    let mut lambda = x.clamp(-crate::math::PI, crate::math::PI);
    let mut phi = y.clamp(-crate::math::HALF_PI, crate::math::HALF_PI);
    let h = 1e-8;
    for _ in 0..40 {
        let (fx, fy) = project(lambda, phi);
        let ex = fx - x;
        let ey = fy - y;
        if ex.abs() < EPSILON2 && ey.abs() < EPSILON2 {
            return Some((lambda, phi));
        }
        // Finite-difference Jacobian
        let (fxl, fyl) = project(lambda + h, phi);
        let (fxp, fyp) = project(lambda, phi + h);
        let a = (fxl - fx) / h;
        let b = (fxp - fx) / h;
        let c = (fyl - fy) / h;
        let d = (fyp - fy) / h;
        let det = a * d - b * c;
        if det.abs() < 1e-15 || !det.is_finite() {
            return None;
        }
        let dl = (ex * d - ey * b) / det;
        let dp = (ey * a - ex * c) / det;
        lambda -= dl;
        phi -= dp;
        if !lambda.is_finite() || !phi.is_finite() {
            return None;
        }
        phi = phi.clamp(-crate::math::HALF_PI, crate::math::HALF_PI);
        lambda = lambda.clamp(-crate::math::PI, crate::math::PI);
    }
    let (fx, fy) = project(lambda, phi);
    if (fx - x).abs() < 1e-6 && (fy - y).abs() < 1e-6 {
        Some((lambda, phi))
    } else {
        None
    }
}

/// Convenience: forward-project spherical degrees through a raw.
pub fn project_degrees(raw: &dyn RawProjection, lon: f64, lat: f64) -> (f64, f64) {
    raw.project(lon * RADIANS, lat * RADIANS)
}

/// Convenience: invert unit-scale planar coordinates to spherical degrees.
pub fn invert_to_degrees(raw: &dyn RawProjection, x: f64, y: f64) -> Option<(f64, f64)> {
    raw.invert(x, y)
        .map(|(lambda, phi)| (lambda * DEGREES, phi * DEGREES))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{HALF_PI, PI};

    fn round_trip(kind: ProjectionKind, tolerance: f64) {
        let raw = kind.raw();
        let mut lambda = -PI + 0.05;
        while lambda < PI {
            let mut phi = -HALF_PI + 0.05;
            while phi < HALF_PI {
                let (x, y) = raw.project(lambda, phi);
                assert!(
                    x.is_finite() && y.is_finite(),
                    "{kind:?} produced non-finite at ({lambda}, {phi})"
                );
                let (l2, p2) = raw
                    .invert(x, y)
                    .unwrap_or_else(|| panic!("{kind:?} invert failed at ({lambda}, {phi})"));
                assert!(
                    (l2 - lambda).abs() < tolerance && (p2 - phi).abs() < tolerance,
                    "{kind:?} round trip ({lambda}, {phi}) -> ({l2}, {p2})"
                );
                phi += 0.31;
            }
            lambda += 0.37;
        }
    }

    #[test]
    fn round_trips() {
        round_trip(ProjectionKind::Equirectangular, 1e-9);
        round_trip(ProjectionKind::Mercator, 1e-9);
        round_trip(ProjectionKind::EqualEarth, 1e-8);
        round_trip(ProjectionKind::NaturalEarth1, 1e-8);
        round_trip(ProjectionKind::WinkelTripel, 1e-5);
        round_trip(ProjectionKind::albers(), 1e-9);
        round_trip(
            ProjectionKind::ConicConformal {
                parallels: (35.0, 65.0),
            },
            1e-8,
        );
        round_trip(ProjectionKind::Identity { reflect_y: true }, 1e-12);
    }

    #[test]
    fn serde_round_trip() {
        let kind = ProjectionKind::albers();
        let json = serde_json::to_string(&kind).unwrap();
        let back: ProjectionKind = serde_json::from_str(&json).unwrap();
        assert_eq!(kind, back);
    }
}
