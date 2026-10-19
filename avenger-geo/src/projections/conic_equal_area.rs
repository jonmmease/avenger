//! Conic equal-area (Albers when paired with the standard parallels).
//!
//! Ported from d3-geo `src/projection/conicEqualArea.js` and
//! `src/projection/cylindricalEqualArea.js` (ISC),
//! <https://github.com/d3/d3-geo>

use super::{Parallels, ProjectionImpl};
use crate::error::AvengerGeoError;
use crate::math::{asin, sign, EPSILON, PI, RADIANS};
use serde::{Deserialize, Serialize};

/// Conic equal-area projection with two standard parallels in degrees.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(from = "Parallels", into = "Parallels")]
pub struct ConicEqualArea {
    parallels: [f64; 2],
    form: Form,
}

#[derive(Debug, Clone, Copy)]
enum Form {
    Conic {
        n: f64,
        c: f64,
        r0: f64,
    },
    /// Degenerate case when the parallels are symmetric about the equator.
    Cylindrical {
        cos_phi0: f64,
    },
}

impl ConicEqualArea {
    pub fn new(parallels: [f64; 2]) -> Self {
        let [y0, y1] = parallels.map(|parallel| parallel * RADIANS);
        let sy0 = y0.sin();
        let n = (sy0 + y1.sin()) / 2.0;
        let form = if n.abs() < EPSILON {
            Form::Cylindrical { cos_phi0: y0.cos() }
        } else {
            let c = 1.0 + sy0 * (2.0 * n - sy0);
            Form::Conic {
                n,
                c,
                r0: c.sqrt() / n,
            }
        };
        ConicEqualArea { parallels, form }
    }

    /// Albers: the standard parallels 29.5° and 45.5°. Rotate it by
    /// `[96, 0, 0]` for the continental US (d3 `geoAlbers`).
    pub fn albers() -> Self {
        Self::new([29.5, 45.5])
    }

    pub fn parallels(&self) -> [f64; 2] {
        self.parallels
    }
}

impl From<Parallels> for ConicEqualArea {
    fn from(Parallels { parallels }: Parallels) -> Self {
        Self::new(parallels)
    }
}

impl From<ConicEqualArea> for Parallels {
    fn from(projection: ConicEqualArea) -> Self {
        Parallels {
            parallels: projection.parallels,
        }
    }
}

#[typetag::serde(name = "conic_equal_area")]
impl ProjectionImpl for ConicEqualArea {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        match self.form {
            Form::Conic { n, c, r0 } => {
                let r = (c - 2.0 * n * phi.sin()).max(0.0).sqrt() / n;
                let x = lambda * n;
                (r * x.sin(), r0 - r * x.cos())
            }
            Form::Cylindrical { cos_phi0 } => (lambda * cos_phi0, phi.sin() / cos_phi0),
        }
    }

    fn validate(&self) -> Result<(), AvengerGeoError> {
        Parallels::validate(self.parallels)
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        match self.form {
            Form::Conic { n, c, r0 } => {
                let r0y = r0 - y;
                let mut l = x.atan2(r0y.abs()) * sign(r0y);
                if r0y * n < 0.0 {
                    l -= PI * sign(x) * sign(r0y);
                }
                Some((l / n, asin((c - (x * x + r0y * r0y) * n * n) / (2.0 * n))))
            }
            Form::Cylindrical { cos_phi0 } => Some((x / cos_phi0, asin(y * cos_phi0))),
        }
    }
}
