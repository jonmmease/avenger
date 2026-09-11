//! Ported from d3-geo `src/projection/naturalEarth1.js` (ISC),
//! <https://github.com/d3/d3-geo>

use super::RawProjection;
use crate::math::EPSILON;

#[derive(Debug, Clone, Copy)]
pub struct NaturalEarth1Raw;

impl RawProjection for NaturalEarth1Raw {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let phi2 = phi * phi;
        let phi4 = phi2 * phi2;
        (
            lambda
                * (0.8707 - 0.131979 * phi2
                    + phi4 * (-0.013791 + phi4 * (0.003971 * phi2 - 0.001529 * phi4))),
            phi * (1.007226
                + phi2 * (0.015085 + phi4 * (-0.044475 + 0.028874 * phi2 - 0.005916 * phi4))),
        )
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let mut phi = y;
        let mut i = 25;
        loop {
            let phi2 = phi * phi;
            let phi4 = phi2 * phi2;
            let delta = (phi
                * (1.007226
                    + phi2 * (0.015085 + phi4 * (-0.044475 + 0.028874 * phi2 - 0.005916 * phi4)))
                - y)
                / (1.007226
                    + phi2
                        * (0.015085 * 3.0
                            + phi4
                                * (-0.044475 * 7.0 + 0.028874 * 9.0 * phi2
                                    - 0.005916 * 11.0 * phi4)));
            phi -= delta;
            i -= 1;
            if delta.abs() <= EPSILON || i == 0 {
                break;
            }
        }
        let phi2 = phi * phi;
        Some((
            x / (0.8707
                + phi2
                    * (-0.131979
                        + phi2 * (-0.013791 + phi2 * phi2 * phi2 * (0.003971 - 0.001529 * phi2)))),
            phi,
        ))
    }
}
