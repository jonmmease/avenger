//! Facet guide implementations for row and column faceting.
//!
//! This module contains:
//! - `FacetRowGuide`: Guide for row-based faceting (vertical labels)
//! - `FacetColGuide`: Guide for column-based faceting (horizontal labels)
//! - Shared types and measurement infrastructure

mod col_guide;
mod measurement;
mod row_guide;
mod shared;

pub use col_guide::FacetColGuide;
pub use measurement::AsyncMeasureOverflowFn;
pub use row_guide::FacetRowGuide;
pub use shared::{
    DEFAULT_OVERFLOW_FALLBACK, FacetSource, FacetTitles, aggregate_cached_overflow,
    apply_shared_overflow_coordination, build_partition_for_facet, compute_scale_sharing_for_nested_facet,
    compute_scale_sharing_from_marks, default_overflow_fallback, extend_partition_list,
};
