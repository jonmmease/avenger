//! # avenger-geo
//!
//! A d3-geo-style spherical projection engine: projections, which other
//! crates can add through [`ProjectionImpl`], wrapped by a shared streaming
//! pipeline (three-axis rotation, spherical clipping to the antimeridian, a
//! circle, or a region, adaptive great-circle resampling, planar rectangle
//! clipping), plus fitting, graticules, Arrow array input, and GeoJSON
//! loading.
//!
//! Pipeline architecture ported from [d3-geo](https://github.com/d3/d3-geo)
//! (ISC license); per-module headers note the source files.
//!
//! Chart-independent: no DataFusion or scene-graph dependencies. Projection
//! returns geometry in output units, which [`paths`] turns into the lyon paths
//! and points that scene marks take.

mod clip;
mod collect;
pub mod error;
mod gesture;
pub mod graticule;
pub mod ingest;
mod math;
pub mod paths;
mod polygon_contains;
pub mod projections;
pub mod projector;
mod resample;
mod rotation;
mod rows;
mod sinks;
pub mod stream;
pub mod tiles;
mod walk;

pub use error::AvengerGeoError;
pub use graticule::{Graticule, GraticuleLine, GraticuleLineKind};
pub use polygon_contains::polygon_contains;
pub use projections::ProjectionImpl;
pub use projector::{
    FitSummary, PanMode, Projection, Projector, RegionFrame, SphericalClip, Surface, View,
};
pub use stream::{GeoSink, RecordingSink};
