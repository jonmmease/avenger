//! # avenger-geo
//!
//! A d3-geo-style spherical projection engine: projections, which other
//! crates can add through [`ProjectionImpl`], wrapped by a shared streaming
//! pipeline (three-axis rotation, antimeridian cutting, adaptive great-circle
//! resampling, planar rectangle clipping), plus fitting, graticules, Arrow
//! array input, and GeoJSON loading.
//!
//! Pipeline architecture ported from [d3-geo](https://github.com/d3/d3-geo)
//! (ISC license); per-module headers note the source files.
//!
//! Chart-independent: no DataFusion or scene-graph dependencies. Projection
//! returns lyon paths and `ScalarOrArray` values that scene marks take.

pub mod clip;
pub mod error;
pub mod graticule;
pub mod ingest;
pub mod math;
pub mod polygon_contains;
pub mod projections;
pub mod projector;
pub mod resample;
pub mod rotation;
mod rows;
mod sinks;
pub mod stream;
mod walk;

pub use error::AvengerGeoError;
pub use graticule::{Graticule, GraticuleLine, GraticuleLineKind};
pub use projections::ProjectionImpl;
pub use projector::{FitSummary, Projection, Projector, Surface, View};
pub use rotation::Rotation;
pub use stream::{GeoSink, RecordingSink};
