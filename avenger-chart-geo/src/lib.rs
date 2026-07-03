//! # avenger-chart-geo
//!
//! General geographic map projections for avenger charts: the [`Geo`]
//! coordinate system (Equal Earth, Natural Earth, Winkel Tripel, Albers and
//! other conics, mercator, equirectangular), graticule/sphere guides,
//! fit-to-data view realization, and (in later phases) geo marks, pan/zoom
//! tools, and warped raster tiles.
//!
//! Design reference: `avenger-chart/docs/future-work/geo-coordinate-system.md`;
//! implementation plan: `scratch/geo/`.

pub mod coord;
pub mod guide;
pub mod marks;
pub mod tools;
pub mod udf;
pub mod view;

pub use avenger_geo::raw::ProjectionKind;
pub use coord::{Geo, GraticuleStyle, SphereStyle};
pub use guide::GeoGuide;
pub use udf::{GeoProjectUdf, geo_project_udf_name, register_geo_project_udf};
pub use view::{GeoCoordMeasurement, GeoView, GeoViewport};
