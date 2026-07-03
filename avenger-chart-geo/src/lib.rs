//! # avenger-chart-geo
//!
//! General geographic map projections for avenger charts: the [`Geo`]
//! coordinate system (Equal Earth, Natural Earth, Winkel Tripel, Albers and
//! other conics, mercator, equirectangular), graticule/sphere guides,
//! fit-to-data view realization, geo marks (symbols, great-circle lines,
//! GeoJSON/WKB shapes), pan/zoom tools with an adaptive Web Mercator
//! blend, and warped raster tile layers. `Geo::mercator()` replaced the
//! retired `avenger-chart-webmercator` coordinate system at pixel parity.
//!
//! Design reference: `avenger-chart/docs/future-work/geo-coordinate-system.md`;
//! implementation plan: `scratch/geo/`.

pub mod coord;
pub mod data;
pub mod expr;
pub mod guide;
pub mod marks;
pub mod tiles;
pub mod tools;
pub mod udf;
pub mod view;

pub use avenger_chart_marks::{Line, Rect, Symbol};
pub use avenger_geo::raw::ProjectionKind;
pub use coord::{BlendConfig, Geo, GraticuleStyle, SphereStyle};
pub use data::{geojson_to_record_batch, register_geojson};
pub use guide::GeoGuide;
pub use marks::{
    CompiledGeoLine, CompiledGeoRect, CompiledGeoShape, CompiledGeoSymbol, GeoGeometrySpace,
    GeoPositionChannels, GeoPositionConfig, GeoShape, IntoGeoExpr,
};
pub use tiles::{RasterTileLayer, TileLoadingPolicy};
pub use tools::GeoPanZoom;
pub use udf::{GeoProjectUdf, geo_project_udf_name, register_geo_project_udf};
pub use view::{GeoCoordMeasurement, GeoView, GeoViewport};
