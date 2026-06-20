//! Compatibility re-export for z-index layer helpers.
//!
//! The implementation now lives in `avenger-scenegraph` because render order is
//! scenegraph semantics shared by WGPU, SVG, PDF, and future renderers.

pub use avenger_scenegraph::render_order::{compute_zindex_layers, verify_partitions};
