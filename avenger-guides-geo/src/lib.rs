//! Map guides and marks for Avenger scene graphs: the sphere, which a map's
//! background fills and its outline frames, the graticule's lines and labels,
//! and the marks of a map's data. The sphere draws through an
//! [`avenger_geo::Projector`] into a scene group. A [`Graticule`] fits its
//! projection at each map size and sizes and draws its labels in layout slabs,
//! as the axes of `avenger-guides` do. [`geometry_marks`] draws screen-space
//! GeoArrow geometry as a path mark and a symbol mark.

mod error;
pub mod graticule;
pub mod marks;
pub mod sphere;

pub use error::AvengerGuidesGeoError;
pub use graticule::{Graticule, GraticuleConfig, GraticuleStyle, GraticuleView, MapFrame};
pub use marks::{geometry_marks, GeometryStyle};
pub use sphere::{make_sphere_marks, SphereStyle};
