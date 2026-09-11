//! Planar identity "projection" for pre-projected data (the d3
//! `geoIdentity` analog, doc §9). Input is treated as planar x/y rather
//! than spherical degrees; the pipeline skips spherical stages for it.

use super::RawProjection;

#[derive(Debug, Clone, Copy)]
pub struct IdentityRaw {
    pub reflect_y: bool,
}

impl RawProjection for IdentityRaw {
    fn project(&self, x: f64, y: f64) -> (f64, f64) {
        (x, if self.reflect_y { -y } else { y })
    }

    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        Some((x, if self.reflect_y { -y } else { y }))
    }
}
