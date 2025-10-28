//! Facet context for passing information from facet to subplots
//!
//! This module provides the `FacetContext` struct that encapsulates all information
//! about a subplot's position within a faceted layout. This context is serialized
//! and passed via params to subplots, allowing guides (axes, legends, etc.) to make
//! intelligent decisions about what to show/hide.

use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Axis position for determining edge-based visibility
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisPosition {
    Top,
    Bottom,
    Left,
    Right,
}

/// Context information passed from facet to subplots
///
/// This context is created by the facet mark and serialized into the params
/// map as `__facet_context`. Subplot guides can then deserialize it to
/// determine positioning, visibility, and other facet-specific behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacetContext {
    /// Position in grid (row_index, col_index)
    /// For row-only faceting, col_index is always 0
    pub position: (usize, usize),

    /// Total grid dimensions (num_rows, num_cols)
    /// For row-only faceting, num_cols is always 1
    pub grid_dimensions: (usize, usize),

    /// Which channels are unified by the facet guide (e.g., {"y"} for row faceting, {"x", "y"} for grid faceting)
    /// When a channel is unified, the facet guide shows the axis title at the
    /// outer level, so subplots should suppress their titles for that channel.
    ///
    /// BREAKING CHANGE: Changed from `Option<String>` to `HashSet<String>` to support GridFacet
    /// where both x and y channels can be unified simultaneously.
    pub unified_channels: HashSet<String>,

    /// Per-channel scale sharing status (true = shared, false = independent)
    /// When scales are shared, only edge subplots need to show labels.
    /// When scales are independent, all subplots should show labels since
    /// the values differ between subplots.
    pub scale_sharing: HashMap<String, bool>,
}

impl FacetContext {
    /// Serialize to params map for passing through call stack
    ///
    /// The context is serialized to JSON and stored under the `__facet_context` key.
    /// This allows it to be passed through the existing params infrastructure without
    /// modifying function signatures.
    pub fn to_params(&self) -> IndexMap<String, ScalarValue> {
        let mut params = IndexMap::new();
        if let Ok(json) = serde_json::to_string(self) {
            params.insert("__facet_context".to_string(), ScalarValue::Utf8(Some(json)));
        }
        params
    }

    /// Deserialize from params map
    ///
    /// Attempts to extract and deserialize the `__facet_context` param.
    /// Returns None if the param doesn't exist or deserialization fails.
    pub fn from_params(params: &IndexMap<String, ScalarValue>) -> Option<Self> {
        params.get("__facet_context").and_then(|v| match v {
            ScalarValue::Utf8(Some(json)) => serde_json::from_str(json).ok(),
            _ => None,
        })
    }

    /// Check if a channel is unified by the facet guide
    ///
    /// When a channel is unified (e.g., "y" in row faceting), the facet guide
    /// shows the axis title at the outer level, so subplots should suppress
    /// their titles for that channel.
    pub fn is_channel_unified(&self, channel: &str) -> bool {
        self.unified_channels.contains(channel)
    }

    /// Check if subplot is on relevant edge for given axis position
    ///
    /// For row faceting:
    /// - x-axis at bottom: only bottom row is on edge (row == num_rows - 1)
    /// - x-axis at top: only top row is on edge (row == 0)
    /// - y-axis at left: always on edge (single column)
    /// - y-axis at right: always on edge (single column)
    ///
    /// For column faceting:
    /// - x-axis: always on edge (single row)
    /// - y-axis at left: only leftmost column is on edge (col == 0)
    /// - y-axis at right: only rightmost column is on edge (col == num_cols - 1)
    ///
    /// For grid faceting (row × column):
    /// - Edges determined by both row and column position
    pub fn is_on_relevant_edge(&self, channel: &str, position: AxisPosition) -> bool {
        let (row, col) = self.position;
        let (num_rows, num_cols) = self.grid_dimensions;

        match (channel, position) {
            ("x", AxisPosition::Bottom) => row == num_rows - 1,
            ("x", AxisPosition::Top) => row == 0,
            ("y", AxisPosition::Left) => col == 0,
            ("y", AxisPosition::Right) => col == num_cols - 1,
            _ => true, // Unknown channel/position combinations show everything
        }
    }

    /// Determine if axis title should be shown
    ///
    /// Title visibility rules:
    /// 1. If channel is unified by facet guide, don't show (facet guide shows it)
    /// 2. Otherwise, only show on relevant edge (e.g., bottom x-axis only on bottom row)
    pub fn should_show_title(&self, channel: &str, position: AxisPosition) -> bool {
        // If unified by facet guide, don't show on subplot (shown by facet guide instead)
        if self.is_channel_unified(channel) {
            return false;
        }

        // Otherwise, only show on relevant edge
        self.is_on_relevant_edge(channel, position)
    }

    /// Determine if axis labels should be shown
    ///
    /// Label visibility rules:
    /// 1. If scales are independent, always show (values differ per subplot)
    /// 2. If scales are shared, only show on relevant edge (values are the same)
    pub fn should_show_labels(&self, channel: &str, position: AxisPosition) -> bool {
        let scales_shared = self.scale_sharing.get(channel).copied().unwrap_or(false);
        let is_edge = self.is_on_relevant_edge(channel, position);

        // Independent scales: always show (values differ per subplot)
        // Shared scales: only show on relevant edge
        !scales_shared || is_edge
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_facet_context_serialization() {
        let mut scale_sharing = HashMap::new();
        scale_sharing.insert("x".to_string(), true);
        scale_sharing.insert("y".to_string(), false);

        let mut unified_channels = HashSet::new();
        unified_channels.insert("y".to_string());

        let ctx = FacetContext {
            position: (1, 0),
            grid_dimensions: (3, 1),
            unified_channels: unified_channels.clone(),
            scale_sharing,
        };

        // Test to_params
        let params = ctx.to_params();
        assert!(params.contains_key("__facet_context"));

        // Test from_params
        let restored = FacetContext::from_params(&params).unwrap();
        assert_eq!(restored.position, (1, 0));
        assert_eq!(restored.grid_dimensions, (3, 1));
        assert_eq!(restored.unified_channels, unified_channels);
        assert_eq!(restored.scale_sharing.get("x"), Some(&true));
        assert_eq!(restored.scale_sharing.get("y"), Some(&false));
    }

    #[test]
    fn test_is_channel_unified() {
        let mut unified_channels = HashSet::new();
        unified_channels.insert("y".to_string());

        let ctx = FacetContext {
            position: (0, 0),
            grid_dimensions: (3, 1),
            unified_channels,
            scale_sharing: HashMap::new(),
        };

        assert!(ctx.is_channel_unified("y"));
        assert!(!ctx.is_channel_unified("x"));
        assert!(!ctx.is_channel_unified("r"));
    }

    #[test]
    fn test_is_on_relevant_edge_row_faceting() {
        // Middle row of 3-row faceting
        let ctx = FacetContext {
            position: (1, 0),
            grid_dimensions: (3, 1),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(),
        };

        // X-axis: not on edge for middle row
        assert!(!ctx.is_on_relevant_edge("x", AxisPosition::Bottom));
        assert!(!ctx.is_on_relevant_edge("x", AxisPosition::Top));

        // Y-axis: always on edge for row faceting (single column)
        assert!(ctx.is_on_relevant_edge("y", AxisPosition::Left));
        assert!(ctx.is_on_relevant_edge("y", AxisPosition::Right));

        // Bottom row
        let ctx_bottom = FacetContext {
            position: (2, 0),
            grid_dimensions: (3, 1),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(),
        };
        assert!(ctx_bottom.is_on_relevant_edge("x", AxisPosition::Bottom));
        assert!(!ctx_bottom.is_on_relevant_edge("x", AxisPosition::Top));

        // Top row
        let ctx_top = FacetContext {
            position: (0, 0),
            grid_dimensions: (3, 1),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(),
        };
        assert!(!ctx_top.is_on_relevant_edge("x", AxisPosition::Bottom));
        assert!(ctx_top.is_on_relevant_edge("x", AxisPosition::Top));
    }

    #[test]
    fn test_should_show_title() {
        let mut unified_channels = HashSet::new();
        unified_channels.insert("y".to_string());

        let ctx = FacetContext {
            position: (1, 0), // Middle row
            grid_dimensions: (3, 1),
            unified_channels,
            scale_sharing: HashMap::new(),
        };

        // Y-axis unified: never show title on subplot
        assert!(!ctx.should_show_title("y", AxisPosition::Left));
        assert!(!ctx.should_show_title("y", AxisPosition::Right));

        // X-axis not unified, but middle row: don't show
        assert!(!ctx.should_show_title("x", AxisPosition::Bottom));
        assert!(!ctx.should_show_title("x", AxisPosition::Top));

        // Bottom row context
        let mut unified_channels_bottom = HashSet::new();
        unified_channels_bottom.insert("y".to_string());

        let ctx_bottom = FacetContext {
            position: (2, 0),
            grid_dimensions: (3, 1),
            unified_channels: unified_channels_bottom,
            scale_sharing: HashMap::new(),
        };

        // X-axis on bottom edge: show
        assert!(ctx_bottom.should_show_title("x", AxisPosition::Bottom));
        assert!(!ctx_bottom.should_show_title("x", AxisPosition::Top));
    }

    #[test]
    fn test_should_show_labels() {
        let mut scale_sharing = HashMap::new();
        scale_sharing.insert("x".to_string(), true); // x-scale shared
        scale_sharing.insert("y".to_string(), false); // y-scale independent

        // Middle row
        let ctx = FacetContext {
            position: (1, 0),
            grid_dimensions: (3, 1),
            unified_channels: HashSet::new(),
            scale_sharing: scale_sharing.clone(),
        };

        // X-axis shared, middle row: don't show labels
        assert!(!ctx.should_show_labels("x", AxisPosition::Bottom));
        assert!(!ctx.should_show_labels("x", AxisPosition::Top));

        // Y-axis independent: always show labels
        assert!(ctx.should_show_labels("y", AxisPosition::Left));
        assert!(ctx.should_show_labels("y", AxisPosition::Right));

        // Bottom row
        let ctx_bottom = FacetContext {
            position: (2, 0),
            grid_dimensions: (3, 1),
            unified_channels: HashSet::new(),
            scale_sharing,
        };

        // X-axis shared, bottom edge: show labels
        assert!(ctx_bottom.should_show_labels("x", AxisPosition::Bottom));
        assert!(!ctx_bottom.should_show_labels("x", AxisPosition::Top));

        // Y-axis independent: always show
        assert!(ctx_bottom.should_show_labels("y", AxisPosition::Left));
    }
}
