//! Conic conformal (Lambert).
//!
//! Ported from d3-geo `src/projection/conicConformal.js` (ISC),
//! <https://github.com/d3/d3-geo>

use super::{Mercator, Parallels, ProjectionImpl};
use crate::error::AvengerGeoError;
use crate::math::{sign, tany, EPSILON, HALF_PI, PI, RADIANS};
use serde::{Deserialize, Serialize};

/// Lambert conformal conic projection with two standard parallels in degrees.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(from = "Parallels", into = "Parallels")]
pub struct ConicConformal {
    parallels: [f64; 2],
    form: Form,
}

#[derive(Debug, Clone, Copy)]
enum Form {
    Conic {
        n: f64,
        f: f64,
    },
    /// Degenerate case (n == 0): mercator.
    Mercator,
}

impl ConicConformal {
    pub fn new(parallels: [f64; 2]) -> Self {
        let [y0, y1] = parallels.map(|parallel| parallel * RADIANS);
        let cy0 = y0.cos();
        let n = if y0 == y1 {
            y0.sin()
        } else {
            (cy0 / y1.cos()).ln() / (tany(y1) / tany(y0)).ln()
        };
        let form = if n == 0.0 || !n.is_finite() {
            Form::Mercator
        } else {
            Form::Conic {
                n,
                f: cy0 * tany(y0).powf(n) / n,
            }
        };
        ConicConformal { parallels, form }
    }

    pub fn parallels(&self) -> [f64; 2] {
        self.parallels
    }
}

impl From<Parallels> for ConicConformal {
    fn from(Parallels { parallels }: Parallels) -> Self {
        Self::new(parallels)
    }
}

impl From<ConicConformal> for Parallels {
    fn from(projection: ConicConformal) -> Self {
        Parallels {
            parallels: projection.parallels,
        }
    }
}

#[typetag::serde(name = "conic_conformal")]
impl ProjectionImpl for ConicConformal {
    fn project(&self, lambda: f64, mut phi: f64) -> (f64, f64) {
        match self.form {
            Form::Conic { n, f } => {
                if f > 0.0 {
                    if phi < -HALF_PI + EPSILON {
                        phi = -HALF_PI + EPSILON;
                    }
                } else if phi > HALF_PI - EPSILON {
                    phi = HALF_PI - EPSILON;
                }
                let r = f / tany(phi).powf(n);
                (r * (n * lambda).sin(), f - r * (n * lambda).cos())
            }
            Form::Mercator => Mercator.project(lambda, phi),
        }
    }

    fn validate(&self) -> Result<(), AvengerGeoError> {
        Parallels::validate(self.parallels)
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        match self.form {
            Form::Conic { n, f } => {
                let fy = f - y;
                let r = sign(n) * (x * x + fy * fy).sqrt();
                let mut l = x.atan2(fy.abs()) * sign(fy);
                if fy * n < 0.0 {
                    l -= PI * sign(x) * sign(fy);
                }
                Some((l / n, 2.0 * (f / r).powf(1.0 / n).atan() - HALF_PI))
            }
            Form::Mercator => Mercator.invert(x, y),
        }
    }
}
