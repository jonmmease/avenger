//! Map guides for Avenger scene graphs: the sphere, which a map's background
//! fills and its outline frames, and the graticule's lines and labels. The
//! sphere draws through an [`avenger_geo::Projector`] into a scene group. A
//! [`Graticule`] fits its projection at each map size and sizes and draws its
//! labels in layout slabs, as the axes of `avenger-guides` do.

mod error;
pub mod graticule;
pub mod sphere;

pub use error::AvengerGuidesGeoError;
pub use graticule::{Graticule, GraticuleConfig, GraticuleStyle, MapFrame};
pub use sphere::{make_sphere_marks, SphereStyle};
