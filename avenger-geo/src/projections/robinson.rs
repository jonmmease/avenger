//! Robinson: a compromise projection interpolated from a table.
//!
//! Ported from d3-geo-projection `src/robinson.js` (ISC),
//! <https://github.com/d3/d3-geo-projection>

use super::ProjectionImpl;
use crate::math::{sign, DEGREES, EPSILON2, PI, RADIANS};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Robinson;

/// Robinson's table: the length of each parallel and its distance from the equator, every 5°,
/// with the distances scaled by d3's factor.
const K: [[f64; 2]; 20] = {
    const Y: f64 = 1.593415793900743;
    [
        [0.9986, -0.062 * Y],
        [1.0000, 0.0000 * Y],
        [0.9986, 0.0620 * Y],
        [0.9954, 0.1240 * Y],
        [0.9900, 0.1860 * Y],
        [0.9822, 0.2480 * Y],
        [0.9730, 0.3100 * Y],
        [0.9600, 0.3720 * Y],
        [0.9427, 0.4340 * Y],
        [0.9216, 0.4958 * Y],
        [0.8962, 0.5571 * Y],
        [0.8679, 0.6176 * Y],
        [0.8350, 0.6769 * Y],
        [0.7986, 0.7346 * Y],
        [0.7597, 0.7903 * Y],
        [0.7186, 0.8435 * Y],
        [0.6732, 0.8936 * Y],
        [0.6213, 0.9394 * Y],
        [0.5722, 0.9761 * Y],
        [0.5322, 1.0000 * Y],
    ]
};

#[typetag::serde(name = "robinson")]
impl ProjectionImpl for Robinson {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        let i = (phi.abs() * 36.0 / PI).min(18.0);
        let i0 = i.floor() as usize;
        let di = i - i0 as f64;
        let [ax, ay] = K[i0];
        let [bx, by] = K[i0 + 1];
        let [cx, cy] = K[(i0 + 2).min(19)];
        (
            lambda * (bx + di * (cx - ax) / 2.0 + di * di * (cx - 2.0 * bx + ax) / 2.0),
            sign(phi) * (by + di * (cy - ay) / 2.0 + di * di * (cy - 2.0 * by + ay) / 2.0),
        )
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let mut phi = y * DEGREES;
        let mut i = (phi / 5.0).abs().min(18.0);
        let mut i0 = i.floor().max(0.0) as i64;
        let mut di;
        loop {
            let ay = K[i0 as usize][1];
            let by = K[i0 as usize + 1][1];
            let cy = K[(i0 as usize + 2).min(19)][1];
            let u = cy - ay;
            let v = cy - 2.0 * by + ay;
            let t = 2.0 * (y.abs() - by) / u;
            let c = v / u;
            di = t * (1.0 - c * t * (1.0 - 2.0 * c * t));
            if di >= 0.0 || i0 == 1 {
                phi = if y >= 0.0 { 5.0 } else { -5.0 } * (di + i);
                let mut j = 50;
                loop {
                    i = (phi.abs() / 5.0).min(18.0);
                    i0 = i.floor() as i64;
                    di = i - i0 as f64;
                    let ay = K[i0 as usize][1];
                    let by = K[i0 as usize + 1][1];
                    let cy = K[(i0 as usize + 2).min(19)][1];
                    let delta = sign(y)
                        * (by + di * (cy - ay) / 2.0 + di * di * (cy - 2.0 * by + ay) / 2.0)
                        - y;
                    phi -= delta * DEGREES;
                    j -= 1;
                    if !(delta.abs() > EPSILON2 && j > 0) {
                        break;
                    }
                }
                break;
            }
            i0 -= 1;
            if i0 < 0 {
                break;
            }
        }
        let i0 = i0.max(0) as usize;
        let [ax, _] = K[i0];
        let [bx, _] = K[i0 + 1];
        let [cx, _] = K[(i0 + 2).min(19)];
        Some((
            x / (bx + di * (cx - ax) / 2.0 + di * di * (cx - 2.0 * bx + ax) / 2.0),
            phi * RADIANS,
        ))
    }
}
