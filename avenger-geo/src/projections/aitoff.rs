//! Aitoff: a compromise projection of the azimuthal equidistant hemisphere, stretched.
//!
//! Ported from d3-geo-projection `src/aitoff.js` (ISC),
//! <https://github.com/d3/d3-geo-projection>

use super::ProjectionImpl;
use crate::math::{acos, EPSILON, HALF_PI, PI};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Aitoff;

/// d3-geo-projection `sinci`: x / sin x, and 1 at 0.
fn sinci(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        x / x.sin()
    }
}

/// d3-geo-projection's `sqrt`, which is 0 below 0.
fn sqrt(x: f64) -> f64 {
    if x > 0.0 {
        x.sqrt()
    } else {
        0.0
    }
}

#[typetag::serde(name = "aitoff")]
impl ProjectionImpl for Aitoff {
    fn project(&self, x: f64, y: f64) -> (f64, f64) {
        let cosy = y.cos();
        let x = x / 2.0;
        let sincia = sinci(acos(cosy * x.cos()));
        (2.0 * cosy * x.sin() * sincia, y.sin() * sincia)
    }

    /// Newton's method, inside the ellipse with semi-axes π and π/2. Near the edge, the
    /// iteration can settle away from the point, as d3's does, and then there is no inverse.
    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if x * x + 4.0 * y * y > PI * PI + EPSILON {
            return None;
        }
        let (mut x1, mut y1) = (x, y);
        let mut i = 25;
        loop {
            let sinx = x1.sin();
            let sinx_2 = (x1 / 2.0).sin();
            let cosx_2 = (x1 / 2.0).cos();
            let siny = y1.sin();
            let cosy = y1.cos();
            let sin_2y = (2.0 * y1).sin();
            let sin2y = siny * siny;
            let cos2y = cosy * cosy;
            let sin2x_2 = sinx_2 * sinx_2;
            let c = 1.0 - cos2y * cosx_2 * cosx_2;
            let (e, f) = if c != 0.0 {
                let f = 1.0 / c;
                (acos(cosy * cosx_2) * sqrt(f), f)
            } else {
                (0.0, 0.0)
            };
            let fx = 2.0 * e * cosy * sinx_2 - x;
            let fy = e * siny - y;
            let dxdx = f * (cos2y * sin2x_2 + e * cosy * cosx_2 * sin2y);
            let dxdy = f * (0.5 * sinx * sin_2y - e * 2.0 * siny * sinx_2);
            let dydx = f * 0.25 * (sin_2y * sinx_2 - e * siny * cos2y * sinx);
            let dydy = f * (sin2y * cosx_2 + e * sin2x_2 * cosy);
            let z = dxdy * dydx - dydy * dxdx;
            if z == 0.0 {
                break;
            }
            let dx = (fy * dxdy - fx * dydy) / z;
            let dy = (fx * dydx - fy * dxdx) / z;
            x1 -= dx;
            y1 -= dy;
            i -= 1;
            if !((dx.abs() > EPSILON || dy.abs() > EPSILON) && i > 0) {
                break;
            }
        }
        let (px, py) = self.project(x1, y1);
        let on_sphere = x1.abs() <= PI + EPSILON && y1.abs() <= HALF_PI + EPSILON;
        (on_sphere && (px - x).abs() < EPSILON && (py - y).abs() < EPSILON).then_some((x1, y1))
    }
}
