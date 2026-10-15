//! Projections: pure `(λ, φ) -> (x, y)` functions at unit scale (radians in,
//! dimensionless y-up planar units out), plus inverses.
//!
//! The pipeline (rotation, clipping, resampling, scale/translate) is layered
//! around these by [`crate::projector`]; a projection contains no view state.
//! Other crates add projections by implementing [`ProjectionImpl`].

mod conic_conformal;
mod conic_equal_area;
mod equal_earth;
mod equirectangular;
mod mercator;
mod natural_earth1;
mod winkel_tripel;

pub use conic_conformal::ConicConformal;
pub use conic_equal_area::ConicEqualArea;
pub use equal_earth::EqualEarth;
pub use equirectangular::Equirectangular;
pub use mercator::Mercator;
pub use natural_earth1::NaturalEarth1;
pub use winkel_tripel::WinkelTripel;

use crate::error::AvengerGeoError;
use crate::math::EPSILON2;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// A kind of projection: maps longitude and latitude in radians to unit-scale,
/// y-up planar coordinates.
///
/// The pipeline cuts geometry at the antimeridian, so a projection's edge must
/// be the antimeridian, as it is for cylindrical, pseudocylindrical, and conic
/// projections.
///
/// Projections serialize through [typetag](https://docs.rs/typetag), so that a
/// serialized projection decodes without registration, including projections
/// defined in other crates. An implementation derives `Serialize` and
/// `Deserialize`, marks its `impl` block `#[typetag::serde(name = "...")]` with
/// a unique name, and depends on `serde` and `typetag` itself.
#[typetag::serde(tag = "type", content = "fields")]
pub trait ProjectionImpl: Debug + Send + Sync + 'static {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64);

    /// The `(lambda, phi)` that projects to `(x, y)`, or `None` where there is
    /// none. The default solves [`project`](Self::project) with
    /// [`invert_newton`].
    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        invert_newton(|lambda, phi| self.project(lambda, phi), x, y)
    }

    /// The rectangle `[[x0, y0], [x1, y1]]`, in unit-scale coordinates, that
    /// the projection draws within. It clips the projection, within the clip
    /// extent when there is one. The default, `None`, doesn't clip.
    fn bounds(&self) -> Option<[[f64; 2]; 2]> {
        None
    }

    /// Check the projection's parameters. [`crate::Projection::build`] calls
    /// it, so a decoded projection is checked like a constructed one. The
    /// default accepts every projection.
    fn validate(&self) -> Result<(), AvengerGeoError> {
        Ok(())
    }
}

/// The serialized form of a conic projection: its standard parallels in
/// degrees.
#[derive(Serialize, Deserialize)]
struct Parallels {
    parallels: [f64; 2],
}

impl Parallels {
    fn validate(parallels: [f64; 2]) -> Result<(), AvengerGeoError> {
        if parallels.iter().all(|parallel| parallel.abs() <= 90.0) {
            Ok(())
        } else {
            Err(AvengerGeoError::InvalidConfig(format!(
                "standard parallels must be latitudes from -90 to 90 degrees, not {parallels:?}"
            )))
        }
    }
}

/// Generic 2D Newton inversion with a finite-difference Jacobian.
///
/// The default [`ProjectionImpl::invert`], for projections without a
/// closed-form inverse (Winkel tripel).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{HALF_PI, PI};

    fn round_trip(projection: &dyn ProjectionImpl, tolerance: f64) {
        let mut lambda = -PI + 0.05;
        while lambda < PI {
            let mut phi = -HALF_PI + 0.05;
            while phi < HALF_PI {
                let (x, y) = projection.project(lambda, phi);
                assert!(
                    x.is_finite() && y.is_finite(),
                    "{projection:?} produced non-finite at ({lambda}, {phi})"
                );
                let (l2, p2) = projection
                    .invert(x, y)
                    .unwrap_or_else(|| panic!("{projection:?} invert failed at ({lambda}, {phi})"));
                assert!(
                    (l2 - lambda).abs() < tolerance && (p2 - phi).abs() < tolerance,
                    "{projection:?} round trip ({lambda}, {phi}) -> ({l2}, {p2})"
                );
                phi += 0.31;
            }
            lambda += 0.37;
        }
    }

    #[test]
    fn round_trips() {
        round_trip(&Equirectangular, 1e-9);
        round_trip(&Mercator, 1e-9);
        round_trip(&EqualEarth, 1e-8);
        round_trip(&NaturalEarth1, 1e-8);
        round_trip(&WinkelTripel, 1e-5);
        round_trip(&ConicEqualArea::albers(), 1e-9);
        round_trip(&ConicConformal::new([35.0, 65.0]), 1e-8);
    }

    #[test]
    fn projections_serialize_by_name() {
        let projections: [Box<dyn ProjectionImpl>; 2] =
            [Box::new(Mercator), Box::new(ConicEqualArea::albers())];
        let json = serde_json::to_string(&projections).unwrap();
        assert_eq!(
            json,
            r#"[{"type":"mercator","fields":null},{"type":"conic_equal_area","fields":{"parallels":[29.5,45.5]}}]"#
        );
        let back: Vec<Box<dyn ProjectionImpl>> = serde_json::from_str(&json).unwrap();
        for (projection, back) in projections.iter().zip(&back) {
            assert_eq!(projection.project(1.0, 0.5), back.project(1.0, 0.5));
        }
    }
}
