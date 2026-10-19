//! Conic equidistant.
//!
//! Ported from d3-geo `src/projection/conicEquidistant.js` (ISC),
//! <https://github.com/d3/d3-geo>

use super::{Parallels, ProjectionImpl};
use crate::error::AvengerGeoError;
use crate::math::{sign, EPSILON, PI, RADIANS};
use serde::{Deserialize, Serialize};

/// Conic equidistant projection with two standard parallels in degrees. Distances along every
/// meridian are true.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(from = "Parallels", into = "Parallels")]
pub struct ConicEquidistant {
    parallels: [f64; 2],
    form: Form,
}

#[derive(Debug, Clone, Copy)]
enum Form {
    Conic {
        n: f64,
        g: f64,
    },
    /// Degenerate case when the cone flattens: equirectangular.
    Equirectangular,
}

impl ConicEquidistant {
    pub fn new(parallels: [f64; 2]) -> Self {
        let [y0, y1] = parallels.map(|parallel| parallel * RADIANS);
        let cy0 = y0.cos();
        let n = if y0 == y1 {
            y0.sin()
        } else {
            (cy0 - y1.cos()) / (y1 - y0)
        };
        let form = if n.abs() < EPSILON {
            Form::Equirectangular
        } else {
            Form::Conic { n, g: cy0 / n + y0 }
        };
        ConicEquidistant { parallels, form }
    }

    pub fn parallels(&self) -> [f64; 2] {
        self.parallels
    }
}

impl From<Parallels> for ConicEquidistant {
    fn from(Parallels { parallels }: Parallels) -> Self {
        Self::new(parallels)
    }
}

impl From<ConicEquidistant> for Parallels {
    fn from(projection: ConicEquidistant) -> Self {
        Parallels {
            parallels: projection.parallels,
        }
    }
}

#[typetag::serde(name = "conic_equidistant")]
impl ProjectionImpl for ConicEquidistant {
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64) {
        match self.form {
            Form::Conic { n, g } => {
                let gy = g - phi;
                let nx = n * lambda;
                (gy * nx.sin(), g - gy * nx.cos())
            }
            Form::Equirectangular => (lambda, phi),
        }
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        match self.form {
            Form::Conic { n, g } => {
                let gy = g - y;
                let mut l = x.atan2(gy.abs()) * sign(gy);
                if gy * n < 0.0 {
                    l -= PI * sign(x) * sign(gy);
                }
                Some((l / n, g - sign(n) * (x * x + gy * gy).sqrt()))
            }
            Form::Equirectangular => Some((x, y)),
        }
    }

    fn validate(&self) -> Result<(), AvengerGeoError> {
        Parallels::validate(self.parallels)
    }
}
