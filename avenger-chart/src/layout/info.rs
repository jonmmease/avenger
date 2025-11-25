//! Layout information produced by marks during evaluation
//!
//! This module defines the `LayoutUpdates` struct for layout data that marks can produce.
//! Unlike `PlotGeometry`, LayoutUpdates is not serializable - it's only used during
//! the rendering process and passed via function parameters.
//!
//! # Usage
//!
//! Most marks return `LayoutUpdates::default()` (empty). Layout marks like Facet return
//! `LayoutUpdates` containing modified scales and overflow measurements.
//!
//! # Design Philosophy
//!
//! Layout updates should ONLY be used by **layout marks** that:
//! - Have a single instance per plot/layer
//! - Have global layout responsibility
//! - Measure content to determine dimensions
//!
//! Examples: Facet, Sankey, Treemap, Force-directed graph
//!
//! **Do NOT use for regular data marks** (Symbol, Line, Rect, etc.)

use crate::guide::OverflowSpaceRequirement;
use crate::scales::ConfiguredScaleWithSpec;
use std::collections::HashMap;

/// Layout updates produced by marks during evaluation
///
/// This struct contains all layout-related data that marks can return, including:
/// - Updated scales (e.g., facet dimension scales with adjusted spacing)
/// - Per-facet overflow measurements for row facets
/// - Per-facet overflow measurements for column facets
///
/// Most marks return `LayoutUpdates::default()`. Only layout marks like Facet
/// populate these fields.
#[derive(Debug, Clone, Default)]
pub struct LayoutUpdates {
    /// Updated scales from layout marks (e.g., facet spacing adjustments)
    ///
    /// When a layout mark measures content and needs to adjust scales, it returns
    /// the updated scales here. These are merged into the plot's scale map.
    pub scales: HashMap<String, ConfiguredScaleWithSpec>,

    /// Per-facet overflow measurements for row facets
    ///
    /// Each element corresponds to one row subplot's overflow (axes, legends, etc.).
    /// Used by guides to calculate facet-level overflow (subplots + facet labels + unified titles).
    ///
    /// Only populated by row facet marks. None for other marks.
    pub row_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,

    /// Per-facet overflow measurements for column facets
    ///
    /// Each element corresponds to one column subplot's overflow (axes, legends, etc.).
    /// Used by guides to calculate facet-level overflow (subplots + facet labels + unified titles).
    ///
    /// Only populated by column facet marks. None for other marks.
    pub col_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
}

impl LayoutUpdates {
    /// Create new layout updates with specific values
    pub fn new(
        scales: HashMap<String, ConfiguredScaleWithSpec>,
        row_overflow: Option<Vec<OverflowSpaceRequirement>>,
        col_overflow: Option<Vec<OverflowSpaceRequirement>>,
    ) -> Self {
        Self {
            scales,
            row_overflow_by_facet: row_overflow,
            col_overflow_by_facet: col_overflow,
        }
    }

    /// Create empty layout updates
    pub fn empty() -> Self {
        Self::default()
    }

    /// Create layout updates with only scales
    pub fn with_scales(scales: HashMap<String, ConfiguredScaleWithSpec>) -> Self {
        Self {
            scales,
            row_overflow_by_facet: None,
            col_overflow_by_facet: None,
        }
    }

    /// Create layout updates with only row overflow
    pub fn with_row_overflow(overflow: Vec<OverflowSpaceRequirement>) -> Self {
        Self {
            scales: HashMap::new(),
            row_overflow_by_facet: Some(overflow),
            col_overflow_by_facet: None,
        }
    }

    /// Create layout updates with only column overflow
    pub fn with_col_overflow(overflow: Vec<OverflowSpaceRequirement>) -> Self {
        Self {
            scales: HashMap::new(),
            row_overflow_by_facet: None,
            col_overflow_by_facet: Some(overflow),
        }
    }

    /// Builder: Add scales
    pub fn add_scales(mut self, scales: HashMap<String, ConfiguredScaleWithSpec>) -> Self {
        self.scales.extend(scales);
        self
    }

    /// Builder: Add row overflow
    pub fn add_row_overflow(mut self, overflow: Vec<OverflowSpaceRequirement>) -> Self {
        self.row_overflow_by_facet = Some(overflow);
        self
    }

    /// Builder: Add column overflow
    pub fn add_col_overflow(mut self, overflow: Vec<OverflowSpaceRequirement>) -> Self {
        self.col_overflow_by_facet = Some(overflow);
        self
    }
}

/// Merge multiple layout updates into a single update
///
/// This function combines layout updates from multiple marks:
/// - Scales are merged (later updates override earlier ones)
/// - Row overflow: takes the LAST non-None value
/// - Column overflow: takes the LAST non-None value
///
/// This is used when multiple marks are evaluated and their layout updates need to be combined.
pub fn merge_layout_updates(layout_updates: &[LayoutUpdates]) -> LayoutUpdates {
    let mut merged_scales = HashMap::new();
    let mut merged_row_overflow: Option<Vec<OverflowSpaceRequirement>> = None;
    let mut merged_col_overflow: Option<Vec<OverflowSpaceRequirement>> = None;

    for update in layout_updates {
        // Merge scales (later updates override earlier ones)
        merged_scales.extend(update.scales.clone());

        // Take last non-None overflow values
        if update.row_overflow_by_facet.is_some() {
            merged_row_overflow = update.row_overflow_by_facet.clone();
        }
        if update.col_overflow_by_facet.is_some() {
            merged_col_overflow = update.col_overflow_by_facet.clone();
        }
    }

    LayoutUpdates {
        scales: merged_scales,
        row_overflow_by_facet: merged_row_overflow,
        col_overflow_by_facet: merged_col_overflow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_empty() {
        let updates = LayoutUpdates::default();
        assert!(updates.scales.is_empty());
        assert!(updates.row_overflow_by_facet.is_none());
        assert!(updates.col_overflow_by_facet.is_none());
    }

    #[test]
    fn test_with_scales() {
        let mut scales = HashMap::new();
        scales.insert("x".to_string(), create_dummy_scale());

        let updates = LayoutUpdates::with_scales(scales.clone());
        assert_eq!(updates.scales.len(), 1);
        assert!(updates.row_overflow_by_facet.is_none());
    }

    #[test]
    fn test_with_row_overflow() {
        let overflow = vec![OverflowSpaceRequirement {
            top: 1.0,
            bottom: 2.0,
            left: 3.0,
            right: 4.0,
        }];

        let updates = LayoutUpdates::with_row_overflow(overflow);
        assert!(updates.scales.is_empty());
        assert_eq!(updates.row_overflow_by_facet.as_ref().unwrap().len(), 1);
        assert!(updates.col_overflow_by_facet.is_none());
    }

    #[test]
    fn test_builder_pattern() {
        let overflow = vec![OverflowSpaceRequirement {
            top: 1.0,
            bottom: 2.0,
            left: 3.0,
            right: 4.0,
        }];

        let updates = LayoutUpdates::default()
            .add_row_overflow(overflow.clone())
            .add_col_overflow(overflow.clone());

        assert!(updates.row_overflow_by_facet.is_some());
        assert!(updates.col_overflow_by_facet.is_some());
    }

    #[test]
    fn test_merge_layout_updates() {
        let overflow1 = vec![OverflowSpaceRequirement {
            top: 1.0,
            bottom: 2.0,
            left: 3.0,
            right: 4.0,
        }];

        let overflow2 = vec![OverflowSpaceRequirement {
            top: 5.0,
            bottom: 6.0,
            left: 7.0,
            right: 8.0,
        }];

        let updates1 = LayoutUpdates::with_row_overflow(overflow1);
        let updates2 = LayoutUpdates::with_col_overflow(overflow2.clone());

        let merged = merge_layout_updates(&[updates1, updates2]);

        // Should have row overflow from first and col overflow from second
        assert!(merged.row_overflow_by_facet.is_some());
        assert!(merged.col_overflow_by_facet.is_some());
        assert_eq!(merged.col_overflow_by_facet.unwrap()[0].top, 5.0);
    }

    fn create_dummy_scale() -> ConfiguredScaleWithSpec {
        use crate::scales::{Scale, ScaleDomain, ScaleRange};
        use avenger_scales::scales::linear::LinearScale;
        use datafusion::logical_expr::lit;

        // Create a scale specification
        let spec = Scale::default()
            .domain(ScaleDomain::new_interval(lit(0.0), lit(100.0)))
            .range(ScaleRange::new_interval(lit(0.0), lit(400.0)));

        // Create a configured scale
        let configured_scale = LinearScale::configured((0.0, 100.0), (0.0, 400.0));

        ConfiguredScaleWithSpec::new(spec, configured_scale)
    }
}
