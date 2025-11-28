//! Coordination context for nested facets
//!
//! This module provides the `FacetCoordinationContext` struct for coordinating
//! nested facet behavior. When an outer facet (e.g., FacetColumn) contains an
//! inner facet (e.g., FacetRow), this context enables:
//!
//! - Domain propagation: Outer facet computes inner facet's domain from full data
//! - Guide ownership: Controls which subplots render facet guides (edge-only)
//! - Overflow coordination: Unified row heights across columns
//!
//! Unlike `FacetContext` which tracks a subplot's position within a single facet,
//! `FacetCoordinationContext` coordinates behavior between facet levels.

use crate::channel::config_traits::ScaleSharing;
use crate::guide::OverflowSpaceRequirement;
use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Controls which facet guides a subplot should render
///
/// When nested facets share their dimension scales, guides should only
/// appear on edge subplots (like grid facets). This enum controls that behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideOwnership {
    /// Render all guides for this dimension (default behavior, or Free scale sharing)
    ///
    /// Used when scale sharing is Free and each subplot has independent values.
    #[default]
    Full,

    /// This is an edge subplot - render guides
    ///
    /// For FacetRow inside FacetColumn: rightmost column renders row guides
    /// For FacetColumn inside FacetRow: bottommost row renders column guides
    Edge,

    /// This is an interior subplot - suppress guides
    ///
    /// Guides are rendered by the edge subplot instead.
    Suppress,
}

impl GuideOwnership {
    /// Compute guide ownership based on position and sharing mode
    ///
    /// # Arguments
    /// * `position` - 0-indexed position within the outer facet
    /// * `count` - Total number of subplots in the outer facet
    /// * `scale_sharing` - Scale sharing mode for the inner facet's dimension
    /// * `is_row_facet` - True if inner facet is FacetRow, false for FacetColumn
    pub fn compute(
        position: usize,
        count: usize,
        scale_sharing: ScaleSharing,
        is_row_facet: bool,
    ) -> Self {
        match scale_sharing {
            ScaleSharing::Free => GuideOwnership::Full,
            ScaleSharing::Shared | ScaleSharing::SharedInRow | ScaleSharing::SharedInColumn => {
                // For FacetRow inside FacetColumn: edge is rightmost (last position)
                // For FacetColumn inside FacetRow: edge is bottommost (last position)
                if is_row_facet {
                    // Row facet's guides go on right edge
                    if position == count - 1 {
                        GuideOwnership::Edge
                    } else {
                        GuideOwnership::Suppress
                    }
                } else {
                    // Column facet's guides go on bottom edge
                    if position == count - 1 {
                        GuideOwnership::Edge
                    } else {
                        GuideOwnership::Suppress
                    }
                }
            }
        }
    }
}

/// Serializable representation of domain values
///
/// ScalarValue doesn't implement Serialize/Deserialize, so we convert
/// domain values to JSON strings for serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializableDomainValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
}

impl SerializableDomainValue {
    /// Convert from ScalarValue
    pub fn from_scalar(value: &ScalarValue) -> Self {
        match value {
            ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                SerializableDomainValue::String(s.clone())
            }
            // Handle Utf8View - DataFusion uses this for string views in newer versions
            ScalarValue::Utf8View(Some(s)) => SerializableDomainValue::String(s.clone()),
            ScalarValue::Int8(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::Int16(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::Int32(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::Int64(Some(n)) => SerializableDomainValue::Int(*n),
            ScalarValue::UInt8(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::UInt16(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::UInt32(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::UInt64(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::Float32(Some(n)) => SerializableDomainValue::Float(*n as f64),
            ScalarValue::Float64(Some(n)) => SerializableDomainValue::Float(*n),
            ScalarValue::Boolean(Some(b)) => SerializableDomainValue::Bool(*b),
            _ => SerializableDomainValue::Null,
        }
    }

    /// Convert to ScalarValue (as Utf8 for strings, Float64 for numbers)
    pub fn to_scalar(&self) -> ScalarValue {
        match self {
            SerializableDomainValue::String(s) => ScalarValue::Utf8(Some(s.clone())),
            SerializableDomainValue::Int(n) => ScalarValue::Int64(Some(*n)),
            SerializableDomainValue::Float(f) => ScalarValue::Float64(Some(*f)),
            SerializableDomainValue::Bool(b) => ScalarValue::Boolean(Some(*b)),
            SerializableDomainValue::Null => ScalarValue::Null,
        }
    }
}

/// Serializable version of scale data extents for passing through coordination context
///
/// This enables outer facets to pass pre-computed data extents (min/max or discrete values)
/// to inner facets, allowing shared scale domains to be computed from the full dataset
/// rather than per-column filtered data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializableDataExtents {
    /// Numeric interval: (min, max)
    Interval { min: f64, max: f64 },
    /// Categorical: unique values
    Discrete(Vec<SerializableDomainValue>),
    /// Temporal interval: (min, max) as Unix timestamps
    Temporal { min: i64, max: i64 },
}

impl SerializableDataExtents {
    /// Create from a numeric interval
    pub fn interval(min: f64, max: f64) -> Self {
        Self::Interval { min, max }
    }

    /// Create from temporal interval (timestamps)
    pub fn temporal(min: i64, max: i64) -> Self {
        Self::Temporal { min, max }
    }

    /// Create from discrete values
    pub fn discrete(values: Vec<ScalarValue>) -> Self {
        Self::Discrete(
            values
                .iter()
                .map(SerializableDomainValue::from_scalar)
                .collect(),
        )
    }
}

/// Coordination context for nested facets
///
/// This context is created by an outer facet and passed to inner facets via params.
/// It enables:
/// - Shared domain computation (inner facet uses outer's pre-computed domain)
/// - Guide suppression (only edge subplots render guides)
/// - Unified overflow (consistent row heights across columns)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacetCoordinationContext {
    /// The channel name that should consume this coordination context
    ///
    /// This identifies which facet dimension (e.g., "row" or "column") the domain
    /// is intended for. When a facet receives a coordination context, it should
    /// only use the domain if its channel name matches this field.
    /// This prevents outer facets from incorrectly consuming domains meant for
    /// inner facets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_channel: Option<String>,

    /// Pre-computed domain values for the inner facet's dimension
    ///
    /// When the outer facet has `scale_sharing: Shared` for the inner dimension,
    /// it computes the domain from the FULL dataset and passes it here.
    /// The inner facet uses this instead of computing from filtered data,
    /// which ensures all columns have the same row values (with empty cells
    /// for missing combinations).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_domain: Option<Vec<SerializableDomainValue>>,

    /// Scale sharing mode for the inner facet's dimension
    ///
    /// Determines whether inner facet should use passed domain or compute its own.
    #[serde(default = "default_scale_sharing")]
    pub inner_scale_sharing: ScaleSharing,

    /// Guide ownership for the inner facet
    ///
    /// Controls whether this subplot renders its facet guides (labels/title).
    /// Edge subplots render guides; interior subplots suppress them.
    #[serde(default)]
    pub guide_ownership: GuideOwnership,

    /// Position of this subplot within the outer facet's layout (0-indexed)
    ///
    /// Used for guide ownership decisions and overflow coordination.
    #[serde(default)]
    pub outer_position: usize,

    /// Total count of subplots in the outer facet
    ///
    /// Used for guide ownership decisions (e.g., is this the last column?).
    #[serde(default)]
    pub outer_count: usize,

    /// Per-row overflow measurements across all columns (for nested coordination)
    ///
    /// When outer facet measures all inner facets, it computes max overflow
    /// per row index across columns. This enables consistent row heights.
    /// The index in this Vec corresponds to the row index within each inner facet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_overflow_by_index: Option<Vec<OverflowSpaceRequirement>>,

    /// Flag indicating this facet should use fallback scales for empty cells
    ///
    /// When domain propagation creates subplots for values not in the filtered data,
    /// those "empty cells" need fallback scales to render axes correctly.
    /// The inner facet should build a fallback ScaleBuilder from the full dataset
    /// and use it when the cell-specific data is empty.
    #[serde(default)]
    pub enable_empty_cell_fallback: bool,

    /// Expected inner domain count from full dataset
    ///
    /// This is the count of unique values in the inner facet's domain, computed from
    /// the full dataset (before outer facet filtering). It's used for grid dimension
    /// calculation in SubplotIterator to ensure consistent grid dimensions across
    /// all outer subplots, even when individual columns have fewer data values.
    ///
    /// For example, in FacetColumn[petal_width_bin] with FacetRow[species]:
    /// - The full dataset has 3 species
    /// - Each column may have 1-3 species depending on the data
    /// - This field stores 3, ensuring all columns report grid=(3, num_columns)
    #[serde(default)]
    pub inner_domain_count: usize,

    /// Pre-computed data extents for shared scale channels (e.g., "x", "y")
    ///
    /// When the outer facet computes shared scales from the full dataset, it can
    /// pass the data extents here. The inner facet uses these to ensure consistent
    /// scale domains across all subplots, even when individual cells have limited data.
    ///
    /// Key: channel name (e.g., "x", "y")
    /// Value: data extents (numeric interval, discrete values, or temporal interval)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_data_extents: Option<HashMap<String, SerializableDataExtents>>,

    /// Pre-computed data extents for SharedInRow scale channels, keyed by row value
    ///
    /// For nested facets with SharedInRow mode, the outer facet computes extents
    /// per inner facet row value. This enables cells in the same row to share
    /// scale domains while allowing different rows to have different domains.
    ///
    /// Outer key: serialized row value (JSON string)
    /// Inner key: channel name (e.g., "x", "y")
    /// Inner value: data extents for that row/channel combination
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_data_extents_by_row:
        Option<HashMap<String, HashMap<String, SerializableDataExtents>>>,

    /// Pre-computed data extents for SharedInColumn scale channels
    ///
    /// For nested facets with SharedInColumn mode, the outer facet computes extents
    /// per column (from the outer facet's filtered data). This enables cells in the
    /// same column to share scale domains while allowing different columns to have
    /// different domains.
    ///
    /// Unlike `shared_data_extents_by_row`, these extents are passed directly to
    /// each inner facet subplot since the outer facet already filters by column value.
    /// Each column's coordination context contains only that column's extents.
    ///
    /// Key: channel name (e.g., "x", "y")
    /// Value: data extents for this column/channel combination
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_data_extents_for_column: Option<HashMap<String, SerializableDataExtents>>,

    /// Per-channel scale sharing modes for x/y axes (computed from channel configs)
    ///
    /// This enables measurement to use the same axis visibility decisions as rendering.
    /// Unlike `inner_scale_sharing` which controls the facet dimension channel (row/column),
    /// this field contains the scale sharing modes for data channels like x and y.
    ///
    /// Key: channel name (e.g., "x", "y")
    /// Value: ScaleSharing mode for that channel
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis_scale_sharing: Option<HashMap<String, ScaleSharing>>,

    /// Inner facet's explicit spacing between subplots (if configured)
    ///
    /// When the inner facet has explicit spacing (e.g., `spacing(20.0)`), this is
    /// propagated to the outer facet so it can account for the total inner spacing
    /// when allocating space for each outer subplot.
    ///
    /// The total inner spacing is: (inner_domain_count - 1) * inner_facet_spacing
    /// This must be accounted for when the outer facet computes band sizes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_facet_spacing: Option<f32>,

    /// Per-column overflow measurements across all rows (for nested coordination)
    ///
    /// When outer FacetRow measures all inner FacetColumn subplots, it computes max overflow
    /// per column index across rows. This enables consistent column widths.
    /// The index in this Vec corresponds to the column index within each inner facet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col_overflow_by_index: Option<Vec<OverflowSpaceRequirement>>,

    /// Coordinated spacing values aggregated from child facets
    ///
    /// During measure_pass, inner facets compute their spacing needs and report them in
    /// FacetPass1Result::spacing_needs. The outer facet aggregates these by taking the max
    /// of each named spacing value across all children, then passes the result back here
    /// during render_pass.
    ///
    /// Standard keys:
    /// - "inter_row_gap": Gap between rows (computed by inner row facet)
    /// - "inter_col_gap": Gap between columns (computed by inner column facet)
    /// - "legend_right": Right margin for legend alignment
    /// - "legend_bottom": Bottom margin for legend alignment
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub coordinated_spacing: HashMap<String, f32>,
}

fn default_scale_sharing() -> ScaleSharing {
    ScaleSharing::Free
}

impl Default for FacetCoordinationContext {
    fn default() -> Self {
        Self {
            inner_channel: None,
            inner_domain: None,
            inner_scale_sharing: ScaleSharing::Free,
            guide_ownership: GuideOwnership::Full,
            outer_position: 0,
            outer_count: 0,
            row_overflow_by_index: None,
            enable_empty_cell_fallback: false,
            inner_domain_count: 0,
            shared_data_extents: None,
            shared_data_extents_by_row: None,
            shared_data_extents_for_column: None,
            axis_scale_sharing: None,
            inner_facet_spacing: None,
            col_overflow_by_index: None,
            coordinated_spacing: HashMap::new(),
        }
    }
}

impl FacetCoordinationContext {
    /// Parameter key used to pass this context through params
    pub const PARAM_KEY: &'static str = "__facet_coordination";

    /// Create a new coordination context
    ///
    /// # Arguments
    /// * `inner_channel` - The channel name ("row" or "column") that should consume this context
    /// * `inner_scale_sharing` - Scale sharing mode for the inner facet
    /// * `guide_ownership` - Controls guide rendering for this subplot
    /// * `outer_position` - Position within the outer facet (0-indexed)
    /// * `outer_count` - Total count of subplots in outer facet
    /// * `inner_domain_count` - Expected inner domain size from full dataset
    pub fn new(
        inner_channel: impl Into<String>,
        inner_scale_sharing: ScaleSharing,
        guide_ownership: GuideOwnership,
        outer_position: usize,
        outer_count: usize,
        inner_domain_count: usize,
    ) -> Self {
        Self {
            inner_channel: Some(inner_channel.into()),
            inner_domain: None,
            inner_scale_sharing,
            guide_ownership,
            outer_position,
            outer_count,
            row_overflow_by_index: None,
            enable_empty_cell_fallback: false,
            inner_domain_count,
            shared_data_extents: None,
            shared_data_extents_by_row: None,
            shared_data_extents_for_column: None,
            axis_scale_sharing: None,
            inner_facet_spacing: None,
            col_overflow_by_index: None,
            coordinated_spacing: HashMap::new(),
        }
    }

    /// Builder: Set per-channel scale sharing modes for x/y axes
    ///
    /// This enables measurement to use the same axis visibility decisions as rendering.
    pub fn with_axis_scale_sharing(mut self, sharing: HashMap<String, ScaleSharing>) -> Self {
        self.axis_scale_sharing = Some(sharing);
        self
    }

    /// Builder: Set inner facet's explicit spacing
    ///
    /// When the inner facet has custom spacing configured, this propagates it
    /// to the outer facet so it can account for the total inner spacing.
    pub fn with_inner_facet_spacing(mut self, spacing: f32) -> Self {
        self.inner_facet_spacing = Some(spacing);
        self
    }

    /// Builder: Set inner domain from ScalarValues
    pub fn with_inner_domain(mut self, domain: Vec<ScalarValue>) -> Self {
        self.inner_domain = Some(
            domain
                .iter()
                .map(SerializableDomainValue::from_scalar)
                .collect(),
        );
        self
    }

    /// Builder: Set row overflow by index
    pub fn with_row_overflow(mut self, overflow: Vec<OverflowSpaceRequirement>) -> Self {
        self.row_overflow_by_index = Some(overflow);
        self
    }

    /// Builder: Set column overflow by index
    pub fn with_col_overflow(mut self, overflow: Vec<OverflowSpaceRequirement>) -> Self {
        self.col_overflow_by_index = Some(overflow);
        self
    }

    /// Builder: Set coordinated spacing values from aggregated child spacing needs
    ///
    /// The outer facet aggregates spacing_needs from all children by taking the max
    /// of each key, then passes the result here for inner facets to use during render.
    pub fn with_coordinated_spacing(mut self, spacing: HashMap<String, f32>) -> Self {
        self.coordinated_spacing = spacing;
        self
    }

    /// Get a coordinated spacing value by key
    ///
    /// Returns the aggregated spacing value if it exists, None otherwise.
    pub fn get_coordinated_spacing(&self, key: &str) -> Option<f32> {
        self.coordinated_spacing.get(key).copied()
    }

    /// Builder: Set outer position and count for proper grid coordinate computation
    ///
    /// This is used when the outer facet iterates over subplots and needs to communicate
    /// the current position to inner facets for proper axis label visibility decisions.
    pub fn with_outer_position(mut self, position: usize, count: usize) -> Self {
        self.outer_position = position;
        self.outer_count = count;
        self
    }

    /// Builder: Enable empty cell fallback for domain propagation
    ///
    /// When enabled, the inner facet will build a fallback ScaleBuilder from the
    /// full dataset (before filtering) and use it to provide scales for cells
    /// that have no data after filtering.
    pub fn with_empty_cell_fallback(mut self, enabled: bool) -> Self {
        self.enable_empty_cell_fallback = enabled;
        self
    }

    /// Builder: Set shared data extents for scale channels
    ///
    /// This enables passing pre-computed data extents (min/max) from the outer
    /// facet to ensure inner facets use consistent scale domains based on the
    /// full dataset rather than per-cell filtered data.
    pub fn with_shared_data_extents(
        mut self,
        extents: HashMap<String, SerializableDataExtents>,
    ) -> Self {
        self.shared_data_extents = Some(extents);
        self
    }

    /// Get shared data extents for a channel
    pub fn get_shared_data_extents(&self, channel: &str) -> Option<&SerializableDataExtents> {
        self.shared_data_extents
            .as_ref()
            .and_then(|extents| extents.get(channel))
    }

    /// Builder: Set shared data extents by row for SharedInRow mode
    ///
    /// For channels with SharedInRow mode, this provides per-row extents.
    /// The outer key is a JSON-serialized row value.
    pub fn with_shared_data_extents_by_row(
        mut self,
        extents: HashMap<String, HashMap<String, SerializableDataExtents>>,
    ) -> Self {
        self.shared_data_extents_by_row = Some(extents);
        self
    }

    /// Get shared data extents for a channel at a specific row value
    ///
    /// Used by inner facets with SharedInRow mode to look up extents
    /// based on the current row value.
    pub fn get_shared_data_extents_for_row(
        &self,
        row_key: &str,
        channel: &str,
    ) -> Option<&SerializableDataExtents> {
        self.shared_data_extents_by_row
            .as_ref()
            .and_then(|by_row| by_row.get(row_key))
            .and_then(|extents| extents.get(channel))
    }

    /// Builder: Set shared data extents for column (SharedInColumn mode)
    ///
    /// For channels with SharedInColumn mode, the outer facet computes extents
    /// for each column from its filtered data and passes them directly to each
    /// inner facet subplot.
    pub fn with_shared_data_extents_for_column(
        mut self,
        extents: HashMap<String, SerializableDataExtents>,
    ) -> Self {
        self.shared_data_extents_for_column = Some(extents);
        self
    }

    /// Get shared data extents for a channel in SharedInColumn mode
    ///
    /// Used by inner facets with SharedInColumn mode to look up extents
    /// that were pre-computed by the outer facet for this column.
    pub fn get_shared_data_extents_for_column(
        &self,
        channel: &str,
    ) -> Option<&SerializableDataExtents> {
        self.shared_data_extents_for_column
            .as_ref()
            .and_then(|extents| extents.get(channel))
    }

    /// Serialize to params map for passing through call stack
    ///
    /// The context is serialized to JSON and stored under the `__facet_coordination` key.
    pub fn to_params(&self) -> IndexMap<String, ScalarValue> {
        let mut params = IndexMap::new();
        if let Ok(json) = serde_json::to_string(self) {
            params.insert(Self::PARAM_KEY.to_string(), ScalarValue::Utf8(Some(json)));
        }
        params
    }

    /// Deserialize from params map
    ///
    /// Attempts to extract and deserialize the `__facet_coordination` param.
    /// Returns None if the param doesn't exist or deserialization fails.
    pub fn from_params(params: &IndexMap<String, ScalarValue>) -> Option<Self> {
        params.get(Self::PARAM_KEY).and_then(|v| match v {
            ScalarValue::Utf8(Some(json)) => serde_json::from_str(json).ok(),
            _ => None,
        })
    }

    /// Update outer position in params and return modified params
    ///
    /// This is used by the outer facet to update the coordination context per-subplot
    /// iteration, ensuring inner facets know their position within the overall grid.
    /// If no coordination context exists in params, they are returned unchanged.
    pub fn update_outer_position_in_params(
        params: &IndexMap<String, ScalarValue>,
        position: usize,
        count: usize,
    ) -> IndexMap<String, ScalarValue> {
        if let Some(mut ctx) = Self::from_params(params) {
            ctx.outer_position = position;
            ctx.outer_count = count;
            let mut new_params = params.clone();
            new_params.extend(ctx.to_params());
            new_params
        } else {
            params.clone()
        }
    }

    /// Update shared data extents for column in params and return modified params
    ///
    /// This is used by the outer facet to add per-column extents for SharedInColumn mode.
    /// Each column computes its extents from filtered data and passes them to inner facets.
    /// If no coordination context exists in params, they are returned unchanged.
    pub fn update_shared_data_extents_for_column_in_params(
        params: &IndexMap<String, ScalarValue>,
        extents: HashMap<String, SerializableDataExtents>,
    ) -> IndexMap<String, ScalarValue> {
        if let Some(mut ctx) = Self::from_params(params) {
            ctx.shared_data_extents_for_column = Some(extents);
            let mut new_params = params.clone();
            new_params.extend(ctx.to_params());
            new_params
        } else {
            params.clone()
        }
    }

    /// Get inner domain as ScalarValues
    pub fn get_inner_domain(&self) -> Option<Vec<ScalarValue>> {
        self.inner_domain
            .as_ref()
            .map(|domain| domain.iter().map(|v| v.to_scalar()).collect())
    }

    /// Get inner domain only if the channel matches
    ///
    /// This is the preferred way to access the domain when consuming a coordination
    /// context. It ensures that only the intended facet (identified by channel name)
    /// uses the domain, preventing outer facets from incorrectly consuming domains
    /// meant for inner facets.
    ///
    /// # Arguments
    /// * `channel` - The channel name of the calling facet (e.g., "row" or "column")
    ///
    /// # Returns
    /// Some(domain) if this context's inner_channel matches the provided channel,
    /// None otherwise.
    pub fn get_inner_domain_for_channel(&self, channel: &str) -> Option<Vec<ScalarValue>> {
        // Only return domain if the channel matches
        if self.inner_channel.as_deref() == Some(channel) {
            self.get_inner_domain()
        } else {
            None
        }
    }

    /// Check if guides should be suppressed for this subplot
    pub fn should_suppress_guides(&self) -> bool {
        matches!(self.guide_ownership, GuideOwnership::Suppress)
    }

    /// Check if this subplot should render guides (Full or Edge ownership)
    pub fn should_render_guides(&self) -> bool {
        !self.should_suppress_guides()
    }

    /// Check if inner domain was provided by outer facet
    pub fn has_inner_domain(&self) -> bool {
        self.inner_domain.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guide_ownership_compute_free() {
        // Free scales: all subplots render guides
        assert_eq!(
            GuideOwnership::compute(0, 3, ScaleSharing::Free, true),
            GuideOwnership::Full
        );
        assert_eq!(
            GuideOwnership::compute(1, 3, ScaleSharing::Free, true),
            GuideOwnership::Full
        );
        assert_eq!(
            GuideOwnership::compute(2, 3, ScaleSharing::Free, true),
            GuideOwnership::Full
        );
    }

    #[test]
    fn test_guide_ownership_compute_shared() {
        // Shared scales: only edge (last) subplot renders guides
        assert_eq!(
            GuideOwnership::compute(0, 3, ScaleSharing::Shared, true),
            GuideOwnership::Suppress
        );
        assert_eq!(
            GuideOwnership::compute(1, 3, ScaleSharing::Shared, true),
            GuideOwnership::Suppress
        );
        assert_eq!(
            GuideOwnership::compute(2, 3, ScaleSharing::Shared, true),
            GuideOwnership::Edge
        );
    }

    #[test]
    fn test_coordination_context_serialization() {
        let ctx = FacetCoordinationContext::new(
            "row",
            ScaleSharing::Shared,
            GuideOwnership::Edge,
            2,
            3,
            2,
        )
        .with_inner_domain(vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ]);

        // Test to_params
        let params = ctx.to_params();
        assert!(params.contains_key(FacetCoordinationContext::PARAM_KEY));

        // Test from_params
        let restored = FacetCoordinationContext::from_params(&params).unwrap();
        assert_eq!(restored.inner_channel.as_deref(), Some("row"));
        assert_eq!(restored.inner_scale_sharing, ScaleSharing::Shared);
        assert_eq!(restored.guide_ownership, GuideOwnership::Edge);
        assert_eq!(restored.outer_position, 2);
        assert_eq!(restored.outer_count, 3);
        assert!(restored.inner_domain.is_some());
        assert_eq!(restored.inner_domain.as_ref().unwrap().len(), 2);

        // Test domain round-trip
        let domain = restored.get_inner_domain().unwrap();
        assert_eq!(domain.len(), 2);

        // Test channel-aware domain access
        let domain_for_row = restored.get_inner_domain_for_channel("row");
        assert!(domain_for_row.is_some());
        assert_eq!(domain_for_row.unwrap().len(), 2);

        // Wrong channel should return None
        let domain_for_col = restored.get_inner_domain_for_channel("column");
        assert!(domain_for_col.is_none());
    }

    #[test]
    fn test_guide_suppression() {
        let ctx_suppress = FacetCoordinationContext::new(
            "row",
            ScaleSharing::Shared,
            GuideOwnership::Suppress,
            0,
            3,
            3,
        );
        assert!(ctx_suppress.should_suppress_guides());
        assert!(!ctx_suppress.should_render_guides());

        let ctx_edge = FacetCoordinationContext::new(
            "row",
            ScaleSharing::Shared,
            GuideOwnership::Edge,
            2,
            3,
            3,
        );
        assert!(!ctx_edge.should_suppress_guides());
        assert!(ctx_edge.should_render_guides());

        let ctx_full =
            FacetCoordinationContext::new("row", ScaleSharing::Free, GuideOwnership::Full, 0, 3, 3);
        assert!(!ctx_full.should_suppress_guides());
        assert!(ctx_full.should_render_guides());
    }

    #[test]
    fn test_serializable_domain_values() {
        // Test string
        let s = SerializableDomainValue::from_scalar(&ScalarValue::Utf8(Some("test".to_string())));
        assert!(matches!(s, SerializableDomainValue::String(_)));
        assert_eq!(s.to_scalar(), ScalarValue::Utf8(Some("test".to_string())));

        // Test int
        let i = SerializableDomainValue::from_scalar(&ScalarValue::Int64(Some(42)));
        assert!(matches!(i, SerializableDomainValue::Int(42)));
        assert_eq!(i.to_scalar(), ScalarValue::Int64(Some(42)));

        // Test float
        let f = SerializableDomainValue::from_scalar(&ScalarValue::Float64(Some(3.14)));
        assert!(matches!(f, SerializableDomainValue::Float(_)));
    }
}
