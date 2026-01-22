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
    /// - Level(n): Hierarchical level-based sharing for nested facets
    ///
    /// Backward compatible via custom deserializer that accepts bool (true=Shared, false=Free)
    #[serde(deserialize_with = "deserialize_scale_sharing")]
    pub scale_sharing: HashMap<String, ScaleSharing>,

    /// Channels that are at the GLOBAL edge for their relevant axis position.
    ///
    /// For unified channels (like x in Row>Row>Row nesting), tick labels should only
    /// appear at the absolute edge of the entire grid, not at intermediate edges.
    /// This field tracks which channels are at their global edge position.
    ///
    /// When creating child contexts, a channel stays in this set only if:
    /// 1. It was in the parent's global_edge_channels (or unified_channels if no parent)
    /// 2. The current position is at the local edge for that channel
    ///
    /// If a channel is unified but NOT in global_edge_channels, its labels are hidden.
    #[serde(default)]
    pub global_edge_channels: HashSet<String>,
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
    ///
    /// Note: For error-aware deserialization, use `try_from_params()` instead.
    pub fn from_params(params: &IndexMap<String, ScalarValue>) -> Option<Self> {
        params.get("__facet_context").and_then(|v| match v {
            ScalarValue::Utf8(Some(json)) => serde_json::from_str(json).ok(),
            _ => None,
        })
    }

    /// Deserialize from params map with explicit error handling
    ///
    /// Unlike `from_params()`, this method distinguishes between:
    /// - `Ok(None)`: The param doesn't exist (valid case - no facet context)
    /// - `Ok(Some(ctx))`: Successfully deserialized
    /// - `Err(...)`: The param exists but deserialization failed (indicates data corruption)
    ///
    /// Use this in contexts where deserialization errors should be propagated rather
    /// than silently converted to None.
    pub fn try_from_params(
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Option<Self>, crate::error::AvengerChartError> {
        match params.get("__facet_context") {
            None => Ok(None),
            Some(ScalarValue::Utf8(None)) => Ok(None),
            Some(ScalarValue::Utf8(Some(json))) => {
                serde_json::from_str(json).map(Some).map_err(|e| {
                    crate::error::AvengerChartError::DeserializationError(format!(
                        "Failed to deserialize FacetContext: {}",
                        e
                    ))
                })
            }
            Some(other) => Err(crate::error::AvengerChartError::DeserializationError(
                format!(
                    "Expected Utf8 for FacetContext, got {:?}",
                    other.data_type()
                ),
            )),
        }
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
    /// - If channel is unified AND has shared scale (same-type nesting): only show at GLOBAL edge
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

        // Check if channel is unified (e.g., x-axis in Row>Row nesting)
        let is_unified = self.is_channel_unified(channel);
        let at_global_edge = self.global_edge_channels.contains(channel);

        // Scale is considered shared if it's Shared or Level > 0
        let scale_is_shared = match sharing_mode {
            ScaleSharing::Free => false,
            ScaleSharing::Shared => true,
            ScaleSharing::Level(n) => n > 0,
        };

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "should_show_labels: channel={} position={:?} sharing_mode={:?} is_unified={} scale_is_shared={} at_global_edge={} grid_pos=({},{}) grid_dims=({},{})",
                channel, position, sharing_mode, is_unified, scale_is_shared, at_global_edge, row, col, num_rows, num_cols
            );
        }

        // If channel is unified AND scale is shared AND global edge tracking is active,
        // only show labels at GLOBAL edge.
        // This ensures tick labels appear only next to the unified axis title in same-type nesting.
        // But if scale is Free, show labels on all subplots since they have independent scales.
        // Note: Only apply global edge logic if global_edge_channels is non-empty (tracking is active).
        // An empty global_edge_channels means we're in a context that doesn't track global edges.
        if is_unified && scale_is_shared && !self.global_edge_channels.is_empty() {
            return at_global_edge && self.is_on_relevant_edge(channel, position);
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
            ScaleSharing::Level(n) => {
                // Hierarchical level-based sharing
                // Level(0) = Free: always show labels
                // Level(1+) = Share with parent: use edge logic like Shared
                if n == 0 {
                    true
                } else {
                    self.is_on_relevant_edge(channel, position)
                }
            }
        }
    }

    /// Build a FacetContext using dimension configuration trait methods.
    ///
    /// This generic constructor uses the `FacetDimensionConfig` trait to compute
    /// position and grid dimensions in a dimension-agnostic way:
    /// - Row faceting: position = (cell_idx, parent_other), grid = (num_cells, parent_other_dim)
    /// - Column faceting: position = (parent_other, cell_idx), grid = (parent_other_dim, num_cells)
    ///
    /// # Arguments
    /// * `cell_idx` - Index of the current cell in this dimension (0..num_cells)
    /// * `num_cells` - Total number of cells in this dimension
    /// * `parent_other` - Parent's position in the orthogonal dimension (row for col facet, col for row facet)
    /// * `parent_other_dim` - Parent's grid size in the orthogonal dimension
    /// * `unified_channels` - Set of channels that are unified by this facet guide
    /// * `scale_sharing` - Per-channel scale sharing configuration
    pub fn build_for_dimension<D: crate::facet::dimension_config::FacetDimensionConfig>(
        cell_idx: usize,
        num_cells: usize,
        parent_other: usize,
        parent_other_dim: usize,
        unified_channels: HashSet<String>,
        scale_sharing: HashMap<String, ScaleSharing>,
    ) -> Self {
        FacetContext {
            position: D::build_cell_position(cell_idx, parent_other),
            grid_dimensions: D::build_grid_dimensions(num_cells, parent_other_dim),
            unified_channels,
            scale_sharing,
            global_edge_channels: HashSet::new(),
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
            global_edge_channels: HashSet::new(),
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
            global_edge_channels: HashSet::new(),
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
            global_edge_channels: HashSet::new(),
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
            global_edge_channels: HashSet::new(),
        };
        assert!(ctx_bottom.is_on_relevant_edge("x", AxisPosition::Bottom));
        assert!(!ctx_bottom.is_on_relevant_edge("x", AxisPosition::Top));

        // Top row
        let ctx_top = FacetContext {
            position: (0, 0),
            grid_dimensions: (3, 1),
            unified_channels: HashSet::new(),
            scale_sharing: HashMap::new(),
            global_edge_channels: HashSet::new(),
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
            global_edge_channels: HashSet::new(),
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
            global_edge_channels: HashSet::new(),
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
            global_edge_channels: HashSet::new(),
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
            global_edge_channels: HashSet::new(),
        };

        // X-axis shared, bottom edge: show labels
        assert!(ctx_bottom.should_show_labels("x", AxisPosition::Bottom));
        assert!(!ctx_bottom.should_show_labels("x", AxisPosition::Top));

        // Y-axis independent: always show
        assert!(ctx_bottom.should_show_labels("y", AxisPosition::Left));
    }

    #[test]
    fn test_try_from_params_success() {
        let mut unified_channels = HashSet::new();
        unified_channels.insert("y".to_string());

        let ctx = FacetContext {
            position: (1, 0),
            grid_dimensions: (3, 1),
            unified_channels: unified_channels.clone(),
            scale_sharing: HashMap::new(),
            global_edge_channels: HashSet::new(),
        };

        let params = ctx.to_params();
        let result = FacetContext::try_from_params(&params);
        assert!(result.is_ok());
        let restored = result.unwrap().unwrap();
        assert_eq!(restored.position, (1, 0));
        assert_eq!(restored.grid_dimensions, (3, 1));
    }

    #[test]
    fn test_try_from_params_missing() {
        let params = IndexMap::new();
        let result = FacetContext::try_from_params(&params);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_try_from_params_invalid_json() {
        let mut params = IndexMap::new();
        params.insert(
            "__facet_context".to_string(),
            ScalarValue::Utf8(Some("not valid json".to_string())),
        );
        let result = FacetContext::try_from_params(&params);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Failed to deserialize FacetContext")
        );
    }

    #[test]
    fn test_try_from_params_wrong_type() {
        let mut params = IndexMap::new();
        params.insert("__facet_context".to_string(), ScalarValue::Int32(Some(42)));
        let result = FacetContext::try_from_params(&params);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Expected Utf8"));
    }

    #[test]
    fn test_build_for_dimension() {
        use crate::facet::dimension_config::{ColumnDimensionConfig, RowDimensionConfig};

        let mut unified_channels = HashSet::new();
        unified_channels.insert("y".to_string());
        let mut scale_sharing = HashMap::new();
        scale_sharing.insert("x".to_string(), ScaleSharing::Shared);

        // Test row faceting: cell 1 of 3, parent col 0 of 1
        let row_ctx = FacetContext::build_for_dimension::<RowDimensionConfig>(
            1,
            3,
            0,
            1,
            unified_channels.clone(),
            scale_sharing.clone(),
        );
        assert_eq!(row_ctx.position, (1, 0)); // (row_idx, parent_col)
        assert_eq!(row_ctx.grid_dimensions, (3, 1)); // (num_rows, parent_num_cols)

        // Test column faceting: cell 2 of 4, parent row 0 of 1
        let col_ctx = FacetContext::build_for_dimension::<ColumnDimensionConfig>(
            2,
            4,
            0,
            1,
            unified_channels,
            scale_sharing,
        );
        assert_eq!(col_ctx.position, (0, 2)); // (parent_row, col_idx)
        assert_eq!(col_ctx.grid_dimensions, (1, 4)); // (parent_num_rows, num_cols)
    }
}
