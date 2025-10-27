//! Dimension configuration trait for faceting
//!
//! Provides parameterization of row vs column faceting behavior to enable
//! code sharing between FacetRow and FacetCol implementations.
//!
//! Key differences abstracted:
//! - Channel name ("row" vs "col")
//! - FacetContext position tuple ((row_idx, 0) vs (0, col_idx))
//! - FacetContext grid dimensions ((num_rows, 1) vs (1, num_cols))
//! - Adjacent overflow calculation (bottom+top vs right+left)
//! - FacetDirection enum value

use crate::guide::{FacetDirection, OverflowSpaceRequirement};

/// Configuration trait that abstracts row vs column faceting behavior
pub trait FacetDimensionConfig: Clone + Send + Sync + 'static {
    /// The channel name for this faceting dimension ("row" or "col")
    fn channel_name() -> &'static str;

    /// The facet direction for guide rendering
    fn facet_direction() -> FacetDirection;

    /// Convert a linear index to a FacetContext position tuple
    ///
    /// Row: (index, 0)
    /// Column: (0, index)
    fn index_to_position(index: usize) -> (usize, usize);

    /// Convert a total count to FacetContext grid dimensions
    ///
    /// Row: (total, 1)
    /// Column: (1, total)
    fn count_to_grid_dimensions(total: usize) -> (usize, usize);

    /// Calculate required spacing between adjacent facets based on overflow
    ///
    /// Row: overflow_a.bottom + overflow_b.top
    /// Column: overflow_a.right + overflow_b.left
    fn calculate_adjacent_overflow(
        overflow_a: &OverflowSpaceRequirement,
        overflow_b: &OverflowSpaceRequirement,
    ) -> f32;
}

/// Row faceting dimension configuration
#[derive(Clone, Debug)]
pub struct RowDimensionConfig;

impl FacetDimensionConfig for RowDimensionConfig {
    fn channel_name() -> &'static str {
        "row"
    }

    fn facet_direction() -> FacetDirection {
        FacetDirection::Row
    }

    fn index_to_position(index: usize) -> (usize, usize) {
        (index, 0)
    }

    fn count_to_grid_dimensions(total: usize) -> (usize, usize) {
        (total, 1)
    }

    fn calculate_adjacent_overflow(
        overflow_a: &OverflowSpaceRequirement,
        overflow_b: &OverflowSpaceRequirement,
    ) -> f32 {
        overflow_a.bottom + overflow_b.top
    }
}

/// Column faceting dimension configuration (placeholder for future FacetCol implementation)
#[derive(Clone, Debug)]
pub struct ColDimensionConfig;

impl FacetDimensionConfig for ColDimensionConfig {
    fn channel_name() -> &'static str {
        "col"
    }

    fn facet_direction() -> FacetDirection {
        FacetDirection::Column
    }

    fn index_to_position(index: usize) -> (usize, usize) {
        (0, index)
    }

    fn count_to_grid_dimensions(total: usize) -> (usize, usize) {
        (1, total)
    }

    fn calculate_adjacent_overflow(
        overflow_a: &OverflowSpaceRequirement,
        overflow_b: &OverflowSpaceRequirement,
    ) -> f32 {
        overflow_a.right + overflow_b.left
    }
}
