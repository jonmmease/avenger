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

/// Legend position info for a single subplot used for cross-subplot alignment
///
/// Stores the X/Y positions of legends at each position (right, left, top, bottom).
/// These positions are populated from Taffy layout bounds after layout computation.
#[derive(Debug, Clone, Default)]
pub struct LegendLayoutInfo {
    /// X position of right legends (for alignment across subplots)
    pub right_x: f32,
    /// X position of left legends (for alignment across subplots)
    pub left_x: f32,
    /// Y position of top legends (for alignment across subplots)
    pub top_y: f32,
    /// Y position of bottom legends (for alignment across subplots)
    pub bottom_y: f32,
}

/// Aggregated legend positions across all subplots for cross-subplot alignment
///
/// Used to compute alignment offsets so legends align across faceted subplots.
/// Contains target positions that all legends should align to.
#[derive(Debug, Clone, Default)]
pub struct LegendAlignmentInfo {
    /// Maximum X position of right legends (for row facets: align to rightmost)
    pub max_right_x: f32,
    /// Minimum X position of right legends (for grid facets: align to leftmost to prevent overflow)
    pub min_right_x: f32,
    /// Minimum X position of left legends (target position for alignment)
    pub min_left_x: f32,
    /// Maximum X position of left legends (for grid facets: align to rightmost)
    pub max_left_x: f32,
    /// Minimum Y position of top legends (target position for alignment)
    pub min_top_y: f32,
    /// Maximum Y position of top legends (for grid facets)
    pub max_top_y: f32,
    /// Maximum Y position of bottom legends (target position for alignment)
    pub max_bottom_y: f32,
    /// Minimum Y position of bottom legends (for grid facets)
    pub min_bottom_y: f32,
}

impl LegendAlignmentInfo {
    /// Aggregate legend layout info from multiple subplots to find target alignment positions
    pub fn aggregate(infos: &[LegendLayoutInfo]) -> Self {
        let mut result = Self {
            min_right_x: f32::MAX,
            min_left_x: f32::MAX,
            min_top_y: f32::MAX,
            min_bottom_y: f32::MAX,
            ..Default::default()
        };
        for info in infos {
            // Right legends: track both min and max
            if info.right_x > 0.0 {
                result.max_right_x = result.max_right_x.max(info.right_x);
                result.min_right_x = result.min_right_x.min(info.right_x);
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "  Aggregating right_x={}, current min={}, max={}",
                        info.right_x, result.min_right_x, result.max_right_x
                    );
                }
            }
            // Left legends: track both min and max
            if info.left_x > 0.0 {
                result.min_left_x = result.min_left_x.min(info.left_x);
                result.max_left_x = result.max_left_x.max(info.left_x);
            }
            // Top legends: track both min and max
            if info.top_y > 0.0 {
                result.min_top_y = result.min_top_y.min(info.top_y);
                result.max_top_y = result.max_top_y.max(info.top_y);
            }
            // Bottom legends: track both min and max
            if info.bottom_y > 0.0 {
                result.max_bottom_y = result.max_bottom_y.max(info.bottom_y);
                result.min_bottom_y = result.min_bottom_y.min(info.bottom_y);
            }
        }
        // Reset min values if not set
        if result.min_right_x == f32::MAX {
            result.min_right_x = 0.0;
        }
        if result.min_left_x == f32::MAX {
            result.min_left_x = 0.0;
        }
        if result.min_top_y == f32::MAX {
            result.min_top_y = 0.0;
        }
        if result.min_bottom_y == f32::MAX {
            result.min_bottom_y = 0.0;
        }
        result
    }
}

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
