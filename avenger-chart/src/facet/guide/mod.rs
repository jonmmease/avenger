//! Facet guide implementations for row and column faceting (STUBBED).
//!
//! This module contains:
//! - `FacetRowGuideConfig` / `FacetRowGuide`: Guide for row-based faceting
//! - `FacetColGuideConfig` / `FacetColGuide`: Guide for column-based faceting

mod band_guide_engine;
mod col_guide;
mod row_guide;

pub use col_guide::{FacetColGuide, FacetColGuideConfig};
pub use row_guide::{FacetRowGuide, FacetRowGuideConfig};
