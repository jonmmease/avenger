//! Ported from d3-geo `src/projection/equalEarth.js` (ISC),
//! <https://github.com/d3/d3-geo>

use super::RawProjection;
use crate::math::{asin, EPSILON2};

const A1: f64 = 1.340264;
const A2: f64 = -0.081106;
const A3: f64 = 0.000893;
const A4: f64 = 0.003796;
const ITERATIONS: usize = 12;

fn m() -> f64 {
    3.0_f64.sqrt() / 2.0
}

#[derive(Debug, Clone, Copy)]
pub struct EqualEarthRaw;

impl RawProjection for EqualEarthRaw {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let m = m();
        let l = asin(m * phi.sin());
        let l2 = l * l;
        let l6 = l2 * l2 * l2;
        (
            lambda * l.cos() / (m * (A1 + 3.0 * A2 * l2 + l6 * (7.0 * A3 + 9.0 * A4 * l2))),
            l * (A1 + A2 * l2 + l6 * (A3 + A4 * l2)),
        )
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let m = m();
        let mut l = y;
        let mut l2 = l * l;
        let mut l6 = l2 * l2 * l2;
        for _ in 0..ITERATIONS {
            let fy = l * (A1 + A2 * l2 + l6 * (A3 + A4 * l2)) - y;
            let fpy = A1 + 3.0 * A2 * l2 + l6 * (7.0 * A3 + 9.0 * A4 * l2);
            let delta = fy / fpy;
            l -= delta;
            l2 = l * l;
            l6 = l2 * l2 * l2;
            if delta.abs() < EPSILON2 {
                break;
            }
        }
        Some((
            m * x * (A1 + 3.0 * A2 * l2 + l6 * (7.0 * A3 + 9.0 * A4 * l2)) / l.cos(),
            asin(l.sin() / m),
        ))
    }
}
