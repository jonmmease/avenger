//! Map guides for Avenger scene graphs: the sphere, which a map's background
//! fills and its outline frames, and the graticule's lines and labels. Each
//! builder draws through an [`avenger_geo::Projector`] into a scene group, as
//! `avenger-guides` draws axes and legends.

mod error;
pub mod graticule;
pub mod sphere;

pub use error::AvengerGuidesGeoError;
pub use graticule::{make_graticule_marks, GraticuleConfig, GraticuleStyle, MapSide};
pub use sphere::{make_sphere_marks, SphereStyle};
