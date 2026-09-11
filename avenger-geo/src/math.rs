//! Spherical math helpers.
//!
//! Ported from d3-geo `src/math.js` and `src/cartesian.js` (ISC),
//! <https://github.com/d3/d3-geo>

pub const EPSILON: f64 = 1e-6;
pub const EPSILON2: f64 = 1e-12;
pub const PI: f64 = std::f64::consts::PI;
pub const HALF_PI: f64 = PI / 2.0;
pub const QUARTER_PI: f64 = PI / 4.0;
pub const TAU: f64 = PI * 2.0;
pub const DEGREES: f64 = 180.0 / PI;
pub const RADIANS: f64 = PI / 180.0;

/// asin clamped to [-1, 1] like d3's `asin`.
pub fn asin(x: f64) -> f64 {
    if x > 1.0 {
        HALF_PI
    } else if x < -1.0 {
        -HALF_PI
    } else {
        x.asin()
    }
}

/// acos clamped to [-1, 1] like d3's `acos`.
pub fn acos(x: f64) -> f64 {
    if x > 1.0 {
        0.0
    } else if x < -1.0 {
        PI
    } else {
        x.acos()
    }
}

/// Sign function matching JS `Math.sign` semantics for our uses (0 -> 0).
pub fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// Spherical (radians) to 3D unit cartesian.
pub fn cartesian(lambda: f64, phi: f64) -> [f64; 3] {
    let cos_phi = phi.cos();
    [cos_phi * lambda.cos(), cos_phi * lambda.sin(), phi.sin()]
}

/// 3D cartesian back to spherical [lambda, phi] (radians).
pub fn spherical(c: &[f64; 3]) -> [f64; 2] {
    [c[1].atan2(c[0]), asin(c[2])]
}

pub fn cartesian_dot(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cartesian_cross(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn cartesian_add_in_place(a: &mut [f64; 3], b: &[f64; 3]) {
    a[0] += b[0];
    a[1] += b[1];
    a[2] += b[2];
}

pub fn cartesian_scale(v: &[f64; 3], k: f64) -> [f64; 3] {
    [v[0] * k, v[1] * k, v[2] * k]
}

pub fn cartesian_normalize_in_place(d: &mut [f64; 3]) {
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    d[0] /= l;
    d[1] /= l;
    d[2] /= l;
}

/// True if two spherical points are within EPSILON (d3 `pointEqual`).
pub fn point_equal(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < EPSILON && (a[1] - b[1]).abs() < EPSILON
}

/// Half-angle tangent helper used by conic conformal / mercator families.
pub fn tany(y: f64) -> f64 {
    ((HALF_PI + y) / 2.0).tan()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cartesian_round_trips() {
        for &(lambda, phi) in &[(0.0, 0.0), (1.0, 0.5), (-2.5, -1.0), (3.0, 1.4)] {
            let c = cartesian(lambda, phi);
            let s = spherical(&c);
            assert!((s[0] - lambda).abs() < 1e-12, "lambda {lambda}");
            assert!((s[1] - phi).abs() < 1e-12, "phi {phi}");
        }
    }

    #[test]
    fn clamped_asin_acos() {
        assert_eq!(asin(2.0), HALF_PI);
        assert_eq!(asin(-2.0), -HALF_PI);
        assert_eq!(acos(2.0), 0.0);
        assert_eq!(acos(-2.0), PI);
    }
}
