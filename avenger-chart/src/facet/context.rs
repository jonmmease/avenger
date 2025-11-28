//! Facet context for passing information from facet to subplots
//!
//! This module provides the `FacetContext` struct that encapsulates all information
//! about a subplot's position within a faceted layout. This context is serialized
//! and passed via params to subplots, allowing guides (axes, legends, etc.) to make
//! intelligent decisions about what to show/hide.

use crate::channel::config_traits::ScaleSharing;
use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet};

/// Axis position for determining edge-based visibility
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisPosition {
    Top,
    Bottom,
    Left,
    Right,
}

/// Backward-compatible deserialization helper for scale_sharing field
///
/// Accepts both old format (bool) and new format (ScaleSharing enum)
#[derive(Deserialize)]
#[serde(untagged)]
enum ScaleSharingCompat {
    Bool(bool),
    Mode(ScaleSharing),
}

/// Custom deserializer for scale_sharing field
fn deserialize_scale_sharing<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, ScaleSharing>, D::Error>
where
    D: Deserializer<'de>,
{
    let compat_map: HashMap<String, ScaleSharingCompat> = HashMap::deserialize(deserializer)?;
    Ok(compat_map
        .into_iter()
        .map(|(k, v)| {
            let mode = match v {
                ScaleSharingCompat::Bool(b) => ScaleSharing::from(b),
                ScaleSharingCompat::Mode(m) => m,
            };
            (k, mode)
        })
        .collect())
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

    /// Per-channel scale sharing configuration
    ///
    /// Determines how scales are shared across facets and affects label visibility:
    /// - Shared: One domain for all facets, labels only on edges
    /// - Free: Independent domains per facet, labels on all facets
    /// - SharedInRow: Shared within each row, labels on left/right edges for y, top/bottom for x
    /// - SharedInColumn: Shared within each column, labels on top/bottom edges for x, left/right for y
    ///
    /// Backward compatible via custom deserializer that accepts bool (true=Shared, false=Free)
    #[serde(deserialize_with = "deserialize_scale_sharing")]
    pub scale_sharing: HashMap<String, ScaleSharing>,
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
    /// - Free: always show (values differ per subplot)
    /// - Shared: only show on relevant edge (values are the same)
    /// - SharedInRow: for y-axis, show on left/right edges; for x-axis, use edge logic
    /// - SharedInColumn: for x-axis, show on top/bottom edges; for y-axis, use edge logic
    pub fn should_show_labels(&self, channel: &str, position: AxisPosition) -> bool {
        let sharing_mode = self
            .scale_sharing
            .get(channel)
            .copied()
            .unwrap_or(ScaleSharing::Free);
        let (row, col) = self.position;
        let (num_rows, num_cols) = self.grid_dimensions;

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "should_show_labels: channel={} position={:?} sharing_mode={:?} grid_pos=({},{}) grid_dims=({},{})",
                channel, position, sharing_mode, row, col, num_rows, num_cols
            );
        }

        match sharing_mode {
            ScaleSharing::Free => {
                // Independent scales: always show labels
                true
            }
            ScaleSharing::Shared => {
                // Shared scales: only show on relevant edge
                self.is_on_relevant_edge(channel, position)
            }
            ScaleSharing::SharedInRow => {
                // Scales shared within each row (across columns)
                match (channel, position) {
                    ("y", AxisPosition::Left) => col == 0, // Show y-labels only on left edge
                    ("y", AxisPosition::Right) => col == num_cols - 1, // Show y-labels only on right edge
                    ("x", _) => {
                        // X-axis uses normal edge logic (show on top/bottom edges)
                        self.is_on_relevant_edge(channel, position)
                    }
                    _ => true, // Unknown combinations show labels
                }
            }
            ScaleSharing::SharedInColumn => {
                // Scales shared within each column (across rows)
                match (channel, position) {
                    ("x", AxisPosition::Bottom) => row == num_rows - 1, // Show x-labels only on bottom edge
                    ("x", AxisPosition::Top) => row == 0, // Show x-labels only on top edge
                    ("y", _) => {
                        // Y-axis uses normal edge logic (show on left/right edges)
                        self.is_on_relevant_edge(channel, position)
                    }
                    _ => true, // Unknown combinations show labels
                }
            }
        }
    }

    // ========================================================================
    // Facet axis visibility methods (parallel to x/y axis methods above)
    // ========================================================================

    /// Check if subplot is on relevant edge for facet guide position
    ///
    /// For row facets (labels on left/right of each column):
    /// - Left position: leftmost column is edge (col == 0)
    /// - Right position: rightmost column is edge (col == num_cols - 1)
    ///
    /// For column facets (labels on top/bottom of each row):
    /// - Top position: topmost row is edge (row == 0)
    /// - Bottom position: bottommost row is edge (row == num_rows - 1)
    pub fn is_facet_on_relevant_edge(&self, channel: &str, position: AxisPosition) -> bool {
        let (row, col) = self.position;
        let (num_rows, num_cols) = self.grid_dimensions;

        match (channel, position) {
            // Row facet labels positioned on left/right of column
            ("row", AxisPosition::Left) => col == 0,
            ("row", AxisPosition::Right) => col == num_cols - 1,
            // Column facet labels positioned on top/bottom of row
            ("column", AxisPosition::Top) => row == 0,
            ("column", AxisPosition::Bottom) => row == num_rows - 1,
            _ => true, // Unknown combinations show everything
        }
    }

    /// Determine if facet title should be shown
    ///
    /// Title visibility rules (parallel to x/y axes):
    /// 1. If channel is unified by outer facet, don't show (outer shows it)
    /// 2. Otherwise, only show on relevant edge
    ///
    /// For row facets inside columns: show title only on rightmost column
    /// For column facets inside rows: show title only on bottommost row
    pub fn should_show_facet_title(&self, channel: &str, position: AxisPosition) -> bool {
        // If unified by outer facet guide, don't show on subplot
        if self.is_channel_unified(channel) {
            return false;
        }

        // Only show on relevant edge
        self.is_facet_on_relevant_edge(channel, position)
    }

    /// Determine if facet labels and ticks/rule should be shown
    ///
    /// Label visibility rules (parallel to x/y axes):
    /// - Free: always show (values may differ per subplot due to filtering)
    /// - Shared: only show on relevant edge (same values across all subplots)
    pub fn should_show_facet_labels(&self, channel: &str, position: AxisPosition) -> bool {
        let sharing_mode = self
            .scale_sharing
            .get(channel)
            .copied()
            .unwrap_or(ScaleSharing::Free);

        match sharing_mode {
            ScaleSharing::Free => {
                // Free scales: always show labels (each subplot may have different values)
                true
            }
            ScaleSharing::Shared | ScaleSharing::SharedInRow | ScaleSharing::SharedInColumn => {
                // Shared scales: only show on relevant edge
                self.is_facet_on_relevant_edge(channel, position)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_facet_context_serialization() {
        let mut scale_sharing = HashMap::new();
        scale_sharing.insert("x".to_string(), ScaleSharing::Shared);
        scale_sharing.insert("y".to_string(), ScaleSharing::Free);

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
        assert_eq!(restored.scale_sharing.get("x"), Some(&ScaleSharing::Shared));
        assert_eq!(restored.scale_sharing.get("y"), Some(&ScaleSharing::Free));
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
        scale_sharing.insert("x".to_string(), ScaleSharing::Shared); // x-scale shared
        scale_sharing.insert("y".to_string(), ScaleSharing::Free); // y-scale independent

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

    // ========================================================================
    // Tests for facet axis visibility methods
    // ========================================================================

    #[test]
    fn test_is_facet_on_relevant_edge() {
        // Test grid faceting (3 rows x 3 cols)
        // Middle cell
        let ctx = FacetContext {
            position: (1, 1),
            grid_dimensions: (3, 3),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(),
        };

        // Row facet: middle column not on edge
        assert!(!ctx.is_facet_on_relevant_edge("row", AxisPosition::Left));
        assert!(!ctx.is_facet_on_relevant_edge("row", AxisPosition::Right));

        // Column facet: middle row not on edge
        assert!(!ctx.is_facet_on_relevant_edge("column", AxisPosition::Top));
        assert!(!ctx.is_facet_on_relevant_edge("column", AxisPosition::Bottom));

        // Rightmost column cell
        let ctx_right = FacetContext {
            position: (1, 2),
            grid_dimensions: (3, 3),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(),
        };
        assert!(!ctx_right.is_facet_on_relevant_edge("row", AxisPosition::Left));
        assert!(ctx_right.is_facet_on_relevant_edge("row", AxisPosition::Right));

        // Leftmost column cell
        let ctx_left = FacetContext {
            position: (1, 0),
            grid_dimensions: (3, 3),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(),
        };
        assert!(ctx_left.is_facet_on_relevant_edge("row", AxisPosition::Left));
        assert!(!ctx_left.is_facet_on_relevant_edge("row", AxisPosition::Right));

        // Bottom row cell
        let ctx_bottom = FacetContext {
            position: (2, 1),
            grid_dimensions: (3, 3),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(),
        };
        assert!(!ctx_bottom.is_facet_on_relevant_edge("column", AxisPosition::Top));
        assert!(ctx_bottom.is_facet_on_relevant_edge("column", AxisPosition::Bottom));
    }

    #[test]
    fn test_should_show_facet_title() {
        // Test with unified row channel (outer facet handles title)
        let mut unified_channels = HashSet::new();
        unified_channels.insert("row".to_string());

        let ctx = FacetContext {
            position: (1, 2), // Rightmost column
            grid_dimensions: (3, 3),
            unified_channels,
            scale_sharing: HashMap::new(),
        };

        // Row channel unified: never show title on subplot
        assert!(!ctx.should_show_facet_title("row", AxisPosition::Right));

        // Without unified channel, show only on edge
        let ctx_non_unified = FacetContext {
            position: (1, 2), // Rightmost column
            grid_dimensions: (3, 3),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(),
        };
        assert!(ctx_non_unified.should_show_facet_title("row", AxisPosition::Right));

        // Middle column: don't show
        let ctx_middle = FacetContext {
            position: (1, 1),
            grid_dimensions: (3, 3),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(),
        };
        assert!(!ctx_middle.should_show_facet_title("row", AxisPosition::Right));
    }

    #[test]
    fn test_should_show_facet_labels() {
        // Test with shared row scale
        let mut scale_sharing = HashMap::new();
        scale_sharing.insert("row".to_string(), ScaleSharing::Shared);

        // Middle column with shared row scale
        let ctx = FacetContext {
            position: (1, 1),
            grid_dimensions: (3, 3),
            unified_channels: HashSet::new(),
            scale_sharing: scale_sharing.clone(),
        };

        // Shared: don't show on middle column
        assert!(!ctx.should_show_facet_labels("row", AxisPosition::Right));

        // Rightmost column with shared row scale
        let ctx_right = FacetContext {
            position: (1, 2),
            grid_dimensions: (3, 3),
            unified_channels: HashSet::new(),
            scale_sharing: scale_sharing.clone(),
        };

        // Shared: show on rightmost column
        assert!(ctx_right.should_show_facet_labels("row", AxisPosition::Right));

        // Test with free row scale (default)
        let ctx_free = FacetContext {
            position: (1, 1), // Middle column
            grid_dimensions: (3, 3),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(), // No sharing = Free
        };

        // Free: always show labels
        assert!(ctx_free.should_show_facet_labels("row", AxisPosition::Right));
    }
}
