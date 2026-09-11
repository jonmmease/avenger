//! # avenger-geo
//!
//! A d3-geo-style spherical projection engine: raw projections wrapped by a
//! shared streaming pipeline (three-axis rotation, antimeridian cutting,
//! adaptive great-circle resampling, planar rectangle clipping), plus fit,
//! graticule/sphere generators, projection blending, and geometry sinks.
//!
//! Pipeline architecture ported from [d3-geo](https://github.com/d3/d3-geo)
//! (ISC license); per-module headers note the source files.
//!
//! Chart-independent: no DataFusion/Arrow/scenegraph dependencies.

pub mod blend;
pub mod clip;
pub mod error;
pub mod graticule;
pub mod ingest;
pub mod math;
pub mod polygon_contains;
pub mod projector;
pub mod raw;
pub mod resample;
pub mod rotation;
pub mod sinks;
pub mod stream;
pub mod streamable;

pub use blend::{anchoring_similarity, BlendRaw, CorrectedBlendRaw, PlanarSimilarity};
pub use error::AvengerGeoError;
pub use graticule::Graticule;
pub use projector::{Affine, BoundsSink, Projection, Projector};
pub use raw::{ProjectionKind, RawProjection};
pub use rotation::Rotation;
pub use sinks::{LyonPathSink, PolylineSink};
pub use stream::{GeoStream, RecordingSink};
pub use streamable::{MultiLine, Sphere, Streamable};
