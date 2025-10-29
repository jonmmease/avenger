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
use std::collections::HashSet;

/// Configuration trait that abstracts row vs column faceting behavior
pub trait FacetDimensionConfig: Clone + Send + Sync + 'static {
    /// The channel name for this faceting dimension ("row" or "col")
    fn channel_name() -> &'static str;

    /// The facet direction for guide rendering
    fn facet_direction() -> FacetDirection;

    /// Get the set of channels unified by this facet dimension
    ///
    /// For row faceting: returns {"y"} (y-axis titles shown by facet guide)
    /// For column faceting: returns {"x"} (x-axis titles shown by facet guide)
    /// For grid faceting: returns {"x", "y"} (both axes unified)
    fn unified_channels() -> HashSet<String>;

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

    /// Check if this is row-only faceting (for scale sharing normalization)
    fn is_row_facet() -> bool {
        false
    }

    /// Check if this is column-only faceting (for scale sharing normalization)
    fn is_col_facet() -> bool {
        false
    }
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

    fn unified_channels() -> HashSet<String> {
        let mut channels = HashSet::new();
        channels.insert("y".to_string());
        channels
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

    fn is_row_facet() -> bool {
        true
    }
}

/// Column faceting dimension configuration
#[derive(Clone, Debug)]
pub struct ColDimensionConfig;

impl FacetDimensionConfig for ColDimensionConfig {
    fn channel_name() -> &'static str {
        "col"
    }

    fn facet_direction() -> FacetDirection {
        FacetDirection::Column
    }

    fn unified_channels() -> HashSet<String> {
        let mut channels = HashSet::new();
        channels.insert("x".to_string());
        channels
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

    fn is_col_facet() -> bool {
        true
    }
}

/// Grid faceting dimension configuration
///
/// Manages 2D grid faceting with both row and column dimensions.
/// Not a traditional FacetDimensionConfig since it handles two dimensions simultaneously.
#[derive(Clone, Debug)]
pub struct GridDimensionConfig {
    pub row: RowDimensionConfig,
    pub col: ColDimensionConfig,
}

impl GridDimensionConfig {
    /// Create a new grid dimension config
    pub fn new() -> Self {
        Self {
            row: RowDimensionConfig,
            col: ColDimensionConfig,
        }
    }

    /// Get the set of channels unified by grid faceting (both x and y)
    pub fn unified_channels() -> HashSet<String> {
        let mut channels = HashSet::new();
        channels.insert("x".to_string());
        channels.insert("y".to_string());
        channels
    }
}

impl Default for GridDimensionConfig {
    fn default() -> Self {
        Self::new()
    }
}
