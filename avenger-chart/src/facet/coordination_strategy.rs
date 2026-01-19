//! Coordination strategy for facet measurement passes
//!
//! This module defines the strategy used to determine how facet measurement
//! passes should coordinate spacing between passes.

use std::collections::HashMap;

use super::dimension_config::FacetDimensionConfig;

/// Strategy for coordinating measurement passes between facets
///
/// This enum determines what happens after the initial measurement pass (Pass 1)
/// and before the final render pass. The strategy determines whether re-measurement
/// is needed based on the spacing requirements detected during Pass 1.
///
/// # Variants
///
/// - `Rerun`: Re-run measure_pass with coordinated spacing (nested facets with cross-dimensional gaps)
/// - `UpdateContextOnly`: Update coordination context without re-measurement (standalone facets)
/// - `NoCoordination`: No coordination needed (simple facets)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinationStrategy {
    /// Re-run measure_pass with coordinated spacing
    ///
    /// Used for nested facets where cross-dimensional gaps need coordination.
    /// For example, a FacetColumn containing FacetRow needs to coordinate row gaps.
    Rerun,

    /// Update coordination context only, no re-measurement
    ///
    /// Used for standalone facets that have spacing requirements (like legend alignment)
    /// but don't need to re-measure because there are no inner facets.
    UpdateContextOnly,

    /// No coordination needed
    ///
    /// Used for simple facets with no spacing requirements.
    NoCoordination,
}

impl CoordinationStrategy {
    /// Determine the coordination strategy based on spacing needs
    ///
    /// # Arguments
    /// * `spacing_needs` - Named spacing requirements from Pass 1
    ///
    /// # Type Parameters
    /// * `DimConfig` - Facet dimension configuration (Row or Column)
    ///
    /// # Returns
    /// The appropriate coordination strategy
    pub fn determine<DimConfig: FacetDimensionConfig>(
        spacing_needs: &HashMap<String, f32>,
    ) -> Self {
        // Cross-dimension key depends on facet orientation:
        // - FacetColumn looks for "inter_row_gap" (from nested FacetRow)
        // - FacetRow looks for "inter_col_gap" (from nested FacetColumn)
        let cross_dim_key = if DimConfig::is_col_facet() {
            "inter_row_gap"
        } else {
            "inter_col_gap"
        };

        if spacing_needs.contains_key(cross_dim_key) {
            Self::Rerun
        } else if !spacing_needs.is_empty() {
            Self::UpdateContextOnly
        } else {
            Self::NoCoordination
        }
    }

    /// Whether this strategy requires re-running the measurement pass
    #[allow(dead_code)]
    pub fn requires_remeasurement(&self) -> bool {
        matches!(self, Self::Rerun)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facet::dimension_config::{ColumnDimensionConfig, RowDimensionConfig};

    #[test]
    fn test_no_coordination_when_empty() {
        let spacing = HashMap::new();
        assert_eq!(
            CoordinationStrategy::determine::<RowDimensionConfig>(&spacing),
            CoordinationStrategy::NoCoordination
        );
        assert_eq!(
            CoordinationStrategy::determine::<ColumnDimensionConfig>(&spacing),
            CoordinationStrategy::NoCoordination
        );
    }

    #[test]
    fn test_update_context_only_for_non_cross_dim() {
        let mut spacing = HashMap::new();
        spacing.insert("some_spacing".to_string(), 10.0);

        assert_eq!(
            CoordinationStrategy::determine::<RowDimensionConfig>(&spacing),
            CoordinationStrategy::UpdateContextOnly
        );
        assert_eq!(
            CoordinationStrategy::determine::<ColumnDimensionConfig>(&spacing),
            CoordinationStrategy::UpdateContextOnly
        );
    }

    #[test]
    fn test_rerun_for_cross_dim_gaps() {
        // FacetRow looks for "inter_col_gap"
        let mut row_spacing = HashMap::new();
        row_spacing.insert("inter_col_gap".to_string(), 5.0);
        assert_eq!(
            CoordinationStrategy::determine::<RowDimensionConfig>(&row_spacing),
            CoordinationStrategy::Rerun
        );

        // FacetColumn looks for "inter_row_gap"
        let mut col_spacing = HashMap::new();
        col_spacing.insert("inter_row_gap".to_string(), 5.0);
        assert_eq!(
            CoordinationStrategy::determine::<ColumnDimensionConfig>(&col_spacing),
            CoordinationStrategy::Rerun
        );
    }

    #[test]
    fn test_requires_remeasurement() {
        assert!(CoordinationStrategy::Rerun.requires_remeasurement());
        assert!(!CoordinationStrategy::UpdateContextOnly.requires_remeasurement());
        assert!(!CoordinationStrategy::NoCoordination.requires_remeasurement());
    }
}
