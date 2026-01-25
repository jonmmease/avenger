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
//!
//! # Field Groups
//!
//! The 23 fields in `FacetCoordinationContext` are organized into 8 logical groups:
//!
//! ## Group 1: Basic Inner Facet Info (4 fields)
//! Core identification and configuration for the inner facet dimension.
//! - `inner_channel`: Channel name ("row" or "col")
//! - `inner_domain`: Pre-computed domain values from full dataset
//! - `inner_scale_sharing`: Scale sharing mode for the dimension
//! - `inner_domain_count`: Expected domain count for consistent grid dimensions
//!
//! ## Group 2: Guide Ownership (3 fields)
//! Controls which subplot renders facet labels and titles.
//! - `guide_ownership`: Full/Edge/Suppress decision
//! - `outer_position`: Position within outer facet (0-indexed)
//! - `outer_count`: Total subplots in outer facet
//!
//! ## Group 3: Overflow Coordination (2 fields)
//! Per-row and per-column overflow measurements for consistent sizing.
//! - `row_overflow_by_index`: Max row heights across all columns
//! - `col_overflow_by_index`: Max column widths across all rows
//!
//! ## Group 4: Empty Cell Fallback (2 fields)
//! Fallback scales for cells with no data.
//! - `enable_empty_cell_fallback`: Flag to use fallback scales
//! - `shared_data_extents`: Pre-computed extents for fallback
//!
//! ## Group 5: Removed
//! The `axis_scale_sharing` field was removed - use `channel_sharing_levels` instead.
//!
//! ## Group 6: Level-Based Sharing (5 fields)
//! Support for hierarchical N-level nested facet scale sharing.
//! - `nesting_depth`: Current depth in hierarchy (0 = outermost)
//! - `channel_sharing_levels`: Per-channel sharing levels (u8)
//! - `level_domains`: Level-based domain lookups
//! - `position_path`: Position path through hierarchy for edge detection
//! - `level_counts`: Subplot counts at each level
//!
//! ## Group 7: Spacing Coordination (2 fields)
//! Gap coordination between nested facets.
//! - `inner_facet_spacing`: Inner facet's explicit spacing
//! - `coordinated_spacing`: Aggregated spacing from children
//!
//! ## Group 8: Uniform Free Scaling (4 fields)
//! Support for uniform cell sizing with phantom cells.
//! - `max_inner_cell_count`: Maximum cells for uniform sizing
//! - `enable_uniform_free_scaling`: Enable uniform sizing flag
//! - `phantom_prepend_count`: Prepended phantom count
//! - `inner_band_align`: Band alignment value (0.0-1.0)

use crate::channel::config_traits::ScaleSharing;
use crate::facet::computed_facet_spec::EvaluatedFacetTree;
use std::sync::Arc;

// ============================================================================
// Spacing Key Constants
// ============================================================================
// These constants define the keys used for coordinated spacing in nested facets.
// Using constants instead of string literals prevents typos and enables IDE
// autocomplete/refactoring support.

/// Key for shared left overflow spacing across nested facets
pub const SHARED_OVERFLOW_LEFT: &str = "shared_overflow_left";

/// Key for shared right overflow spacing across nested facets
pub const SHARED_OVERFLOW_RIGHT: &str = "shared_overflow_right";

/// Key for shared top overflow spacing across nested facets
pub const SHARED_OVERFLOW_TOP: &str = "shared_overflow_top";

/// Key for shared bottom overflow spacing across nested facets
pub const SHARED_OVERFLOW_BOTTOM: &str = "shared_overflow_bottom";
use crate::guide::OverflowSpaceRequirement;
use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Key for level-based domain lookups in FacetCoordinationContext
///
/// This struct combines a hierarchy level with a channel name to form a
/// unique key for looking up shared domain values in the level_domains IndexMap.
///
/// # Example
/// ```ignore
/// // Create a key for level 1 y-axis domain
/// let key = LevelChannelKey::new(1, "y");
///
/// // Use it to look up in the coordination context
/// if let Some(domain) = coord_ctx.level_domains.get(&key) {
///     // Use the shared domain
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LevelChannelKey {
    /// The hierarchy level (0 = innermost, higher = further up)
    pub level: usize,
    /// The channel name (e.g., "x", "y", "color")
    pub channel: String,
}

impl LevelChannelKey {
    /// Create a new LevelChannelKey
    pub fn new(level: usize, channel: impl Into<String>) -> Self {
        Self {
            level,
            channel: channel.into(),
        }
    }
}

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

// ============================================================================
// Accessor View Types
// ============================================================================
// These view types provide focused access to logical groups of fields in
// FacetCoordinationContext, making it clear which fields are needed for
// specific operations.

/// View into guide-related fields for visibility decisions
///
/// Use this when you need to determine whether to render guides (axes, labels, titles).
/// The view provides access to guide ownership and position information without
/// exposing unrelated coordination fields.
#[derive(Debug, Clone, Copy)]
pub struct GuideContextView {
    /// Controls whether this subplot renders guides
    pub ownership: GuideOwnership,
    /// Position within the outer facet (0-indexed)
    pub outer_position: usize,
    /// Total count of subplots in the outer facet
    pub outer_count: usize,
}

impl GuideContextView {
    /// Check if guides should be suppressed for this subplot
    pub fn should_suppress(&self) -> bool {
        matches!(self.ownership, GuideOwnership::Suppress)
    }

    /// Check if this subplot should render guides (Full or Edge ownership)
    pub fn should_render(&self) -> bool {
        !self.should_suppress()
    }

    /// Check if this is the last (edge) position in the outer facet
    pub fn is_edge_position(&self) -> bool {
        self.outer_count > 0 && self.outer_position == self.outer_count - 1
    }
}

/// View into overflow coordination fields
///
/// Use this when aggregating overflow measurements across nested facets.
/// Provides access to per-row and per-column overflow values without
/// exposing unrelated coordination fields.
#[derive(Debug, Clone)]
pub struct OverflowContextView<'a> {
    /// Per-row overflow measurements (max heights across all columns)
    pub row_overflow: Option<&'a Vec<OverflowSpaceRequirement>>,
    /// Per-column overflow measurements (max widths across all rows)
    pub col_overflow: Option<&'a Vec<OverflowSpaceRequirement>>,
}

impl<'a> OverflowContextView<'a> {
    /// Get the overflow for a specific row index
    pub fn row_at(&self, index: usize) -> Option<&OverflowSpaceRequirement> {
        self.row_overflow.and_then(|v| v.get(index))
    }

    /// Get the overflow for a specific column index
    pub fn col_at(&self, index: usize) -> Option<&OverflowSpaceRequirement> {
        self.col_overflow.and_then(|v| v.get(index))
    }
}

/// Axis position for determining edge-based visibility
///
/// Re-export from context module for use in coordination context
pub use crate::facet::context::AxisPosition;

/// Serializable representation of domain values
///
/// ScalarValue doesn't implement Serialize/Deserialize, so we convert
/// domain values to this enum for serialization through coordination context.
///
/// This enum preserves type information for grouping and domain operations,
/// with variants for all common Arrow/DataFusion scalar types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SerializableDomainValue {
    String(String),
    Int(i64),
    /// Unsigned 64-bit integer - stored separately to avoid i64 overflow
    UInt64(u64),
    Float(f64),
    Bool(bool),
    /// Decimal128 stored as string to preserve precision
    /// Format: "value:precision:scale" (e.g., "12345:10:2" for 123.45)
    Decimal128(String),
    /// Timestamp in milliseconds since Unix epoch
    TimestampMs(i64),
    /// Timestamp in microseconds since Unix epoch
    TimestampUs(i64),
    /// Timestamp in nanoseconds since Unix epoch
    TimestampNs(i64),
    Null,
}

impl SerializableDomainValue {
    /// Convert from ScalarValue
    ///
    /// Handles all common Arrow scalar types, preserving type information
    /// for proper round-trip serialization.
    pub fn from_scalar(value: &ScalarValue) -> Self {
        match value {
            // String types
            ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                SerializableDomainValue::String(s.clone())
            }
            // Handle Utf8View - DataFusion uses this for string views in newer versions
            ScalarValue::Utf8View(Some(s)) => SerializableDomainValue::String(s.clone()),

            // Signed integer types
            ScalarValue::Int8(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::Int16(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::Int32(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::Int64(Some(n)) => SerializableDomainValue::Int(*n),

            // Unsigned integer types - small ones fit in i64
            ScalarValue::UInt8(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::UInt16(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::UInt32(Some(n)) => SerializableDomainValue::Int(*n as i64),
            // UInt64 uses dedicated variant to avoid overflow
            ScalarValue::UInt64(Some(n)) => SerializableDomainValue::UInt64(*n),

            // Float types
            ScalarValue::Float32(Some(n)) => SerializableDomainValue::Float(*n as f64),
            ScalarValue::Float64(Some(n)) => SerializableDomainValue::Float(*n),

            // Boolean
            ScalarValue::Boolean(Some(b)) => SerializableDomainValue::Bool(*b),

            // Decimal128 - serialize as string to preserve precision
            ScalarValue::Decimal128(Some(value), precision, scale) => {
                SerializableDomainValue::Decimal128(format!("{}:{}:{}", value, precision, scale))
            }

            // Timestamp types - preserve the time unit in variant
            ScalarValue::TimestampMillisecond(Some(ts), _) => {
                SerializableDomainValue::TimestampMs(*ts)
            }
            ScalarValue::TimestampMicrosecond(Some(ts), _) => {
                SerializableDomainValue::TimestampUs(*ts)
            }
            ScalarValue::TimestampNanosecond(Some(ts), _) => {
                SerializableDomainValue::TimestampNs(*ts)
            }
            ScalarValue::TimestampSecond(Some(ts), _) => {
                // Convert seconds to milliseconds for consistent storage
                SerializableDomainValue::TimestampMs(*ts * 1000)
            }

            // Date types - convert to milliseconds
            ScalarValue::Date32(Some(days)) => {
                // days since Unix epoch -> milliseconds
                SerializableDomainValue::TimestampMs(*days as i64 * 86_400_000)
            }
            ScalarValue::Date64(Some(ms)) => SerializableDomainValue::TimestampMs(*ms),

            // Dictionary types - unwrap to the underlying value
            // This handles Arrow Dictionary-encoded columns which are common for categorical data
            ScalarValue::Dictionary(_, inner) => {
                // Recursively convert the inner value
                Self::from_scalar(inner.as_ref())
            }

            // Everything else becomes Null
            _ => SerializableDomainValue::Null,
        }
    }

    /// Convert to ScalarValue for use in domain operations
    ///
    /// This enables round-trip: ScalarValue -> SerializableDomainValue -> ScalarValue
    pub fn to_scalar(&self) -> ScalarValue {
        match self {
            SerializableDomainValue::String(s) => ScalarValue::Utf8(Some(s.clone())),
            SerializableDomainValue::Int(n) => ScalarValue::Int64(Some(*n)),
            SerializableDomainValue::UInt64(n) => ScalarValue::UInt64(Some(*n)),
            SerializableDomainValue::Float(f) => ScalarValue::Float64(Some(*f)),
            SerializableDomainValue::Bool(b) => ScalarValue::Boolean(Some(*b)),
            SerializableDomainValue::Decimal128(s) => {
                // Parse "value:precision:scale" format
                let parts: Vec<&str> = s.split(':').collect();
                if parts.len() == 3 {
                    if let (Ok(value), Ok(precision), Ok(scale)) = (
                        parts[0].parse::<i128>(),
                        parts[1].parse::<u8>(),
                        parts[2].parse::<i8>(),
                    ) {
                        return ScalarValue::Decimal128(Some(value), precision, scale);
                    }
                }
                // Fallback to null if parsing fails
                ScalarValue::Null
            }
            SerializableDomainValue::TimestampMs(ts) => {
                ScalarValue::TimestampMillisecond(Some(*ts), None)
            }
            SerializableDomainValue::TimestampUs(ts) => {
                ScalarValue::TimestampMicrosecond(Some(*ts), None)
            }
            SerializableDomainValue::TimestampNs(ts) => {
                ScalarValue::TimestampNanosecond(Some(*ts), None)
            }
            SerializableDomainValue::Null => ScalarValue::Null,
        }
    }
}

/// Serializable version of scale data extents for passing through coordination context
///
/// This enables outer facets to pass pre-computed data extents (min/max or discrete values)
/// to inner facets, allowing shared scale domains to be computed from the full dataset
/// rather than per-column filtered data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SerializableDataExtents {
    /// Numeric interval: (min, max)
    Interval { min: f64, max: f64 },
    /// Numeric interval with radius-aware padding info for symbols.
    /// Stores the max radius values so domain expansion can be computed correctly
    /// when sharing domains across cells with radius-aware scales.
    RadiusAwareInterval {
        min: f64,
        max: f64,
        max_radius_lower: f64,
        max_radius_upper: f64,
    },
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

    /// Create from a numeric interval with radius-aware padding info
    pub fn radius_aware_interval(
        min: f64,
        max: f64,
        max_radius_lower: f64,
        max_radius_upper: f64,
    ) -> Self {
        Self::RadiusAwareInterval {
            min,
            max,
            max_radius_lower,
            max_radius_upper,
        }
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
/// This context is created by an outer facet and passed to inner facets directly.
/// It enables:
/// - Shared domain computation (inner facet uses outer's pre-computed domain)
/// - Guide suppression (only edge subplots render guides)
/// - Unified overflow (consistent row heights across columns)
#[derive(Debug, Clone)]
pub struct FacetCoordinationContext {
    /// Reference to the evaluated facet tree for efficient domain lookups.
    ///
    /// Non-optional for consistency with RenderContext and simpler accessors.
    /// Use `EvaluatedFacetTree::empty()` for non-faceted cases.
    pub facet_tree: Arc<EvaluatedFacetTree>,

    /// The channel name that should consume this coordination context
    ///
    /// This identifies which facet dimension (e.g., "row" or "column") the domain
    /// is intended for. When a facet receives a coordination context, it should
    /// only use the domain if its channel name matches this field.
    /// This prevents outer facets from incorrectly consuming domains meant for
    /// inner facets.
    pub inner_channel: Option<String>,

    /// Pre-computed domain values for the inner facet's dimension
    ///
    /// When the outer facet has `scale_sharing: Shared` for the inner dimension,
    /// it computes the domain from the FULL dataset and passes it here.
    /// The inner facet uses this instead of computing from filtered data,
    /// which ensures all columns have the same row values (with empty cells
    /// for missing combinations).
    pub inner_domain: Option<Vec<SerializableDomainValue>>,

    /// Scale sharing mode for the inner facet's dimension
    ///
    /// Determines whether inner facet should use passed domain or compute its own.
    pub inner_scale_sharing: ScaleSharing,

    /// Guide ownership for the inner facet
    ///
    /// Controls whether this subplot renders its facet guides (labels/title).
    /// Edge subplots render guides; interior subplots suppress them.
    pub guide_ownership: GuideOwnership,

    /// Position of this subplot within the outer facet's layout (0-indexed)
    ///
    /// Used for guide ownership decisions and overflow coordination.
    pub outer_position: usize,

    /// Total count of subplots in the outer facet
    ///
    /// Used for guide ownership decisions (e.g., is this the last column?).
    pub outer_count: usize,

    /// The channel type of the outer facet (e.g., "row" or "column")
    ///
    /// Used to detect same-type nesting (Row>Row or Col>Col) vs cross-type nesting.
    /// In same-type nesting, the orthogonal dimension is always 1.
    /// In cross-type nesting, outer_position represents the orthogonal dimension.
    pub outer_channel: Option<String>,

    /// Per-row overflow measurements across all columns (for nested coordination)
    ///
    /// When outer facet measures all inner facets, it computes max overflow
    /// per row index across columns. This enables consistent row heights.
    /// The index in this Vec corresponds to the row index within each inner facet.
    pub row_overflow_by_index: Option<Vec<OverflowSpaceRequirement>>,

    /// Flag indicating this facet should use fallback scales for empty cells
    ///
    /// When domain propagation creates subplots for values not in the filtered data,
    /// those "empty cells" need fallback scales to render axes correctly.
    /// The inner facet should build a fallback ScaleBuilder from the full dataset
    /// and use it when the cell-specific data is empty.
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
    pub inner_domain_count: usize,

    /// Pre-computed data extents for shared scale channels (e.g., "x", "y")
    ///
    /// When the outer facet computes shared scales from the full dataset, it can
    /// pass the data extents here. The inner facet uses these to ensure consistent
    /// scale domains across all subplots, even when individual cells have limited data.
    ///
    /// Key: channel name (e.g., "x", "y")
    /// Value: DomainExtent (numeric/discrete/temporal bounds with optional radius)
    pub shared_data_extents: Option<HashMap<String, crate::scales::DomainExtent>>,

    // ========================================================================
    // Level-based scale sharing fields (Phase 2)
    // ========================================================================
    /// Current nesting depth in the facet hierarchy
    ///
    /// 0 = outermost facet (no parent facets)
    /// 1 = first nested level (inside one parent facet)
    /// N = Nth nested level (inside N parent facets)
    ///
    /// Used to determine which level domains are relevant for scale sharing.
    pub nesting_depth: usize,

    /// Per-channel scale sharing levels (converted from ScaleSharing)
    ///
    /// Maps channel names to their sharing level:
    /// - 0: Free (independent per cell)
    /// - 1: Share with immediate parent
    /// - N: Share N levels up
    /// - u8::MAX: Shared (global across all facets)
    ///
    /// This is derived from ScaleSharing via to_level() for efficient lookups.
    pub channel_sharing_levels: HashMap<String, u8>,

    /// Level-based domain lookups
    ///
    /// Maps (level, channel) pairs to pre-computed data extents.
    /// Level 1 domains come from the outer_filtered_df.
    /// Level > nesting_depth domains come from full_df (global scope).
    ///
    /// Key: LevelChannelKey { level, channel }
    /// Value: DomainExtent (numeric/discrete/temporal bounds with optional radius)
    pub level_domains: IndexMap<LevelChannelKey, crate::scales::DomainExtent>,

    /// Position path through the facet hierarchy
    ///
    /// For a 3-level nested structure (L0 > L1 > L2), this might be:
    /// [0, 2, 1] meaning: position 0 in outermost, position 2 in middle, position 1 in innermost
    ///
    /// Used for edge detection and guide ownership decisions.
    pub position_path: Vec<usize>,

    /// Count of subplots at each level in the hierarchy
    ///
    /// For a 3-level nested structure, this might be:
    /// [3, 4, 2] meaning: 3 subplots at outermost, 4 at middle, 2 at innermost
    ///
    /// Used with position_path for edge detection.
    pub level_counts: Vec<usize>,

    /// Inner facet's explicit spacing between subplots (if configured)
    ///
    /// When the inner facet has explicit spacing (e.g., `spacing(20.0)`), this is
    /// propagated to the outer facet so it can account for the total inner spacing
    /// when allocating space for each outer subplot.
    ///
    /// The total inner spacing is: (inner_domain_count - 1) * inner_facet_spacing
    /// This must be accounted for when the outer facet computes band sizes.
    pub inner_facet_spacing: Option<f32>,

    /// Per-column overflow measurements across all rows (for nested coordination)
    ///
    /// When outer FacetRow measures all inner FacetColumn subplots, it computes max overflow
    /// per column index across rows. This enables consistent column widths.
    /// The index in this Vec corresponds to the column index within each inner facet.
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
    pub coordinated_spacing: HashMap<String, f32>,

    // ========================================================================
    // Uniform Free scaling fields
    // ========================================================================
    /// Maximum inner cell count for uniform Free scaling sizing
    ///
    /// When nested facets use ScaleSharing::Free, different parent cells can have
    /// different numbers of child subplots. This field stores the maximum count
    /// across all parent cells, enabling uniform subplot sizes.
    ///
    /// Set during measure_pass when nested facet detection finds Free scaling.
    /// Used by guides to compute band dimensions based on max count rather than
    /// local count.
    pub max_inner_cell_count: Option<usize>,

    /// Enable uniform cell sizing for Free scaling
    ///
    /// When true, inner facets use max_inner_cell_count for band sizing instead
    /// of their local domain count. This ensures all subplots have uniform sizes
    /// with empty space where data is missing.
    ///
    /// Only has effect when max_inner_cell_count is Some.
    pub enable_uniform_free_scaling: bool,

    /// Number of phantom cells prepended for uniform Free scaling
    ///
    /// When uniform sizing adds phantom cells and they're prepended (band_align >= 0.5),
    /// this stores how many phantoms were added at the start. This offset is used by
    /// SubplotIterator to correctly compute FacetContext.position for axis label visibility.
    ///
    /// 0 = No phantoms, or phantoms were appended (not prepended)
    /// N = N phantoms were prepended, so actual data starts at rendered position N
    pub phantom_prepend_count: usize,

    /// Band alignment for the inner facet dimension (0.0 to 1.0)
    ///
    /// Used by guides to compute phantom_prepend_count locally when they don't know
    /// it from the outer facet. This enables correct axis label visibility positioning
    /// for nested facets with uniform Free scaling.
    ///
    /// - 0.0: Data aligns to start (top for rows, left for columns), phantoms appended
    /// - 0.5: Data centered, phantoms split evenly
    /// - 1.0: Data aligns to end (bottom for rows, right for columns), phantoms prepended
    pub inner_band_align: f32,

    // ========================================================================
    // Grammar-based partition model (Phase 1)
    // ========================================================================
    /// Explicit partition list for grammar-based visibility derivation.
    ///
    /// This field captures the grammar's `partitions: [Partition]` model, where
    /// each partition represents a facet variable with:
    /// - field: The data field being partitioned on
    /// - direction: Row or Column
    /// - domain_sharing: How domain values are shared (0=nest, 255=cross)
    /// - domain_values: The ordered domain values
    ///
    /// When present, this enables structure-derived visibility decisions rather
    /// than incremental edge tracking. See the partition module for details.
    pub partition_list: Option<crate::facet::partition::FacetPartitionList>,

    /// Pre-computed visibility decisions for all subplots (Phase 3 grammar model)
    ///
    /// This cache computes visibility once for all subplots based on the grammar's
    /// rules, rather than computing incrementally during iteration.
    pub visibility_cache: Option<crate::facet::partition::VisibilityCache>,
}

impl Default for FacetCoordinationContext {
    fn default() -> Self {
        Self {
            facet_tree: Arc::new(EvaluatedFacetTree::empty()),
            inner_channel: None,
            inner_domain: None,
            inner_scale_sharing: ScaleSharing::Free,
            guide_ownership: GuideOwnership::Full,
            outer_position: 0,
            outer_count: 0,
            outer_channel: None,
            row_overflow_by_index: None,
            enable_empty_cell_fallback: false,
            inner_domain_count: 0,
            shared_data_extents: None,
            // Level-based scale sharing fields
            nesting_depth: 0,
            channel_sharing_levels: HashMap::new(),
            level_domains: IndexMap::new(),
            position_path: Vec::new(),
            level_counts: Vec::new(),
            // End level-based scale sharing fields
            inner_facet_spacing: None,
            col_overflow_by_index: None,
            coordinated_spacing: HashMap::new(),
            // Uniform Free scaling fields
            max_inner_cell_count: None,
            enable_uniform_free_scaling: false,
            phantom_prepend_count: 0,
            inner_band_align: 0.5, // Default to centered
            // Grammar-based partition model
            partition_list: None,
            visibility_cache: None,
        }
    }
}

impl FacetCoordinationContext {
    /// Create a new coordination context
    ///
    /// # Arguments
    /// * `facet_tree` - Reference to the evaluated facet tree
    /// * `inner_channel` - The channel name ("row" or "column") that should consume this context
    /// * `inner_scale_sharing` - Scale sharing mode for the inner facet
    /// * `guide_ownership` - Controls guide rendering for this subplot
    /// * `outer_position` - Position within the outer facet (0-indexed)
    /// * `outer_count` - Total count of subplots in outer facet
    /// * `inner_domain_count` - Expected inner domain size from full dataset
    pub fn new(
        facet_tree: Arc<EvaluatedFacetTree>,
        inner_channel: impl Into<String>,
        inner_scale_sharing: ScaleSharing,
        guide_ownership: GuideOwnership,
        outer_position: usize,
        outer_count: usize,
        inner_domain_count: usize,
    ) -> Self {
        Self {
            facet_tree,
            inner_channel: Some(inner_channel.into()),
            inner_domain: None,
            inner_scale_sharing,
            guide_ownership,
            outer_position,
            outer_count,
            outer_channel: None,
            row_overflow_by_index: None,
            enable_empty_cell_fallback: false,
            inner_domain_count,
            shared_data_extents: None,
            // Level-based scale sharing fields
            nesting_depth: 0,
            channel_sharing_levels: HashMap::new(),
            level_domains: IndexMap::new(),
            position_path: Vec::new(),
            level_counts: Vec::new(),
            // End level-based scale sharing fields
            inner_facet_spacing: None,
            col_overflow_by_index: None,
            coordinated_spacing: HashMap::new(),
            // Uniform Free scaling fields
            max_inner_cell_count: None,
            enable_uniform_free_scaling: false,
            phantom_prepend_count: 0,
            inner_band_align: 0.5, // Default to centered
            // Grammar-based partition model
            partition_list: None,
            visibility_cache: None,
        }
    }

    /// Create an empty context for non-faceted cases
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builder: Set band alignment for inner facet dimension
    ///
    /// This enables guides to compute phantom_prepend_count locally for
    /// correct axis label visibility positioning.
    ///
    /// - 0.0: Data aligns to start (top for rows), phantoms appended
    /// - 0.5: Data centered, phantoms split evenly
    /// - 1.0: Data aligns to end (bottom for rows), phantoms prepended
    pub fn with_inner_band_align(mut self, align: f32) -> Self {
        self.inner_band_align = align;
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

    /// Builder: Set the outer facet's channel type ("row" or "column")
    ///
    /// Used to detect same-type nesting (Row>Row or Col>Col) vs cross-type nesting.
    /// In same-type nesting, the inner facet always has orthogonal dimension of 1.
    /// In cross-type nesting, outer_position represents the orthogonal dimension.
    pub fn with_outer_channel(mut self, channel: impl Into<String>) -> Self {
        self.outer_channel = Some(channel.into());
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

    /// Builder: Set maximum inner cell count for uniform Free scaling
    ///
    /// This is the maximum number of child subplots across all parent cells.
    /// When uniform Free scaling is enabled, all subplots use this count for
    /// band sizing instead of their local domain count.
    ///
    /// The count is floored at 1 to prevent divide-by-zero errors.
    pub fn with_max_inner_cell_count(mut self, count: usize) -> Self {
        self.max_inner_cell_count = Some(count.max(1));
        self
    }

    /// Builder: Enable or disable uniform cell sizing for Free scaling
    ///
    /// When enabled (and max_inner_cell_count is set), inner facets compute
    /// band dimensions based on the maximum cell count rather than their local
    /// domain count, ensuring uniform subplot sizes across all parent cells.
    pub fn with_uniform_free_scaling(mut self, enabled: bool) -> Self {
        self.enable_uniform_free_scaling = enabled;
        self
    }

    /// Get uniform cell count for band sizing if enabled
    ///
    /// Returns Some(count) when:
    /// - enable_uniform_free_scaling is true AND
    /// - max_inner_cell_count is Some
    ///
    /// Returns None otherwise, indicating normal per-cell sizing should be used.
    pub fn get_uniform_cell_count(&self) -> Option<usize> {
        if self.enable_uniform_free_scaling {
            self.max_inner_cell_count
        } else {
            None
        }
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
        extents: HashMap<String, crate::scales::DomainExtent>,
    ) -> Self {
        self.shared_data_extents = Some(extents);
        self
    }

    /// Get shared data extents for a channel
    pub fn get_shared_data_extents(&self, channel: &str) -> Option<&crate::scales::DomainExtent> {
        self.shared_data_extents
            .as_ref()
            .and_then(|extents| extents.get(channel))
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

    // ========================================================================
    // Tree-Derived Accessor Methods
    // ========================================================================
    // These methods derive values from the facet_tree, providing a path to
    // remove redundant fields that duplicate tree-derivable information.

    /// Get the outer path as ScalarValues for tree queries.
    ///
    /// Uses tree traversal by index to recover actual domain values from position_path.
    fn outer_path_values(&self) -> Vec<ScalarValue> {
        self.facet_tree.path_values_from_indices(&self.position_path)
    }

    /// Get nesting depth derived from tree.
    ///
    /// During migration, this includes a debug assertion to verify it matches the field.
    pub fn get_nesting_depth(&self) -> usize {
        let tree_depth = self.facet_tree.depth();
        #[cfg(debug_assertions)]
        {
            // Note: tree depth is 1-based (1 for single facet), nesting_depth is 0-based
            // After migration, tree_depth will be the source of truth
            if self.nesting_depth > 0 && tree_depth > 0 {
                // Only validate when both are set (during migration, field may be set but tree empty)
                // The tree depth should be nesting_depth + 1 (converting from 0-based to 1-based)
                let expected_tree_depth = self.nesting_depth + 1;
                if tree_depth != expected_tree_depth && tree_depth != self.nesting_depth {
                    // Allow either convention during migration
                    debug_assert!(
                        false,
                        "Tree depth mismatch: tree.depth()={} but nesting_depth={}",
                        tree_depth, self.nesting_depth
                    );
                }
            }
        }
        // Return the field value during migration (will return tree_depth after field removal)
        self.nesting_depth
    }

    /// Get level counts derived from tree.
    ///
    /// During migration, this includes a debug assertion to verify it matches the field.
    pub fn get_level_counts(&self) -> Vec<usize> {
        let tree_counts = self.facet_tree.level_counts();
        #[cfg(debug_assertions)]
        {
            if !self.level_counts.is_empty() && !tree_counts.is_empty() {
                debug_assert_eq!(
                    tree_counts, self.level_counts,
                    "Level counts mismatch: tree={:?} but field={:?}",
                    tree_counts, self.level_counts
                );
            }
        }
        // Return the field value during migration
        self.level_counts.clone()
    }

    /// Get inner domain count derived from tree.
    ///
    /// During migration, this includes a debug assertion to verify it matches the field.
    pub fn get_inner_domain_count(&self) -> usize {
        let outer_path = self.outer_path_values();
        let tree_count = self.facet_tree.max_inner_cell_count(&outer_path).unwrap_or(0);
        #[cfg(debug_assertions)]
        {
            if self.inner_domain_count > 0 && tree_count > 0 {
                debug_assert_eq!(
                    tree_count, self.inner_domain_count,
                    "Inner domain count mismatch: tree={} but field={}",
                    tree_count, self.inner_domain_count
                );
            }
        }
        // Return the field value during migration
        self.inner_domain_count
    }

    /// Get max inner cell count derived from tree.
    ///
    /// During migration, this includes a debug assertion to verify it matches the field.
    pub fn get_max_inner_cell_count(&self) -> Option<usize> {
        let outer_path = self.outer_path_values();
        let tree_count = self.facet_tree.max_inner_cell_count(&outer_path);
        #[cfg(debug_assertions)]
        {
            if let (Some(field_count), Some(tree_count)) = (self.max_inner_cell_count, tree_count) {
                debug_assert_eq!(
                    tree_count, field_count,
                    "Max inner cell count mismatch: tree={} but field={}",
                    tree_count, field_count
                );
            }
        }
        // Return the field value during migration
        self.max_inner_cell_count
    }

    // ========================================================================
    // Accessor View Methods
    // ========================================================================
    // These methods provide focused views into logical groups of fields,
    // making it clear which fields are needed for specific operations.

    /// Returns guide-related fields for visibility decisions
    ///
    /// Use this view when determining whether to render guides (axes, labels, titles)
    /// for a subplot. The view encapsulates the ownership mode and position information
    /// needed for edge-based guide rendering.
    ///
    /// # Example
    /// ```ignore
    /// let guide_view = coord_ctx.guide_view();
    /// if guide_view.should_render() {
    ///     // Render guides for this subplot
    /// }
    /// ```
    pub fn guide_view(&self) -> GuideContextView {
        GuideContextView {
            ownership: self.guide_ownership,
            outer_position: self.outer_position,
            outer_count: self.outer_count,
        }
    }

    /// Returns overflow coordination fields
    ///
    /// Use this view when aggregating overflow measurements across nested facets
    /// or when applying coordinated overflow values during rendering.
    ///
    /// # Example
    /// ```ignore
    /// let overflow_view = coord_ctx.overflow_view();
    /// if let Some(overflow) = overflow_view.row_at(row_index) {
    ///     // Use coordinated overflow for this row
    /// }
    /// ```
    pub fn overflow_view(&self) -> OverflowContextView<'_> {
        OverflowContextView {
            row_overflow: self.row_overflow_by_index.as_ref(),
            col_overflow: self.col_overflow_by_index.as_ref(),
        }
    }

    // ========================================================================
    // Level-based scale sharing methods (Phase 2)
    // ========================================================================

    /// Get the sharing level for a channel
    ///
    /// Returns the level value (0-255) from channel_sharing_levels if set,
    /// otherwise returns 0 (Free) as the default.
    ///
    /// # Arguments
    /// * `channel` - Channel name (e.g., "x", "y", "color")
    ///
    /// # Returns
    /// The sharing level: 0 = Free, 1-254 = Level(N), 255 = Shared (global)
    pub fn get_channel_level(&self, channel: &str) -> u8 {
        self.channel_sharing_levels
            .get(channel)
            .copied()
            .unwrap_or(0)
    }

    /// Check if a channel should use the parent facet's domain
    ///
    /// Returns true if the channel's sharing level is >= 1, meaning it should
    /// share scales with at least the immediate parent facet.
    ///
    /// # Arguments
    /// * `channel` - Channel name (e.g., "x", "y")
    ///
    /// # Returns
    /// True if channel should use parent domain, false if independent
    pub fn should_use_parent_domain(&self, channel: &str) -> bool {
        self.get_channel_level(channel) > 0
    }

    /// Get the grammar-based depth (number of partitions in the hierarchy).
    ///
    /// When a partition_list is available, returns its depth directly.
    /// Otherwise, returns nesting_depth + 1 (converting from 0-based to 1-based).
    ///
    /// Grammar semantics: depth = len(partitions)
    /// - Single facet: depth = 1
    /// - Two-level (Row > Col): depth = 2
    /// - Three-level (Row > Col > Row): depth = 3
    pub fn grammar_depth(&self) -> usize {
        if let Some(ref partition_list) = self.partition_list {
            partition_list.depth()
        } else {
            // Fall back to legacy: nesting_depth is 0-based, so add 1
            // nesting_depth=0 means outermost (1 partition), nesting_depth=1 means 2 partitions, etc.
            self.nesting_depth + 1
        }
    }

    /// Compute partition prefix depth for a sharing level using the grammar formula.
    ///
    /// Grammar formula: partition_depth(k) = max(0, depth - k)
    ///
    /// This determines how many leading partitions define a sharing group:
    /// - k=0 (Free): all partitions (depth) - fully nested
    /// - k=1: depth-1 partitions - share with immediate parent
    /// - k=depth (global): 0 partitions - everyone shares
    pub fn partition_depth(&self, sharing_level: u8) -> usize {
        let depth = self.grammar_depth();
        let k = sharing_level as usize;
        depth.saturating_sub(k)
    }

    /// Get the domain for a channel based on its sharing level
    ///
    /// Looks up the domain in level_domains using the channel's sharing level.
    /// The level is clamped to the nesting_depth to avoid looking for domains
    /// beyond what's available in the hierarchy.
    ///
    /// For Level(0) or Free: returns None (use computed domain)
    /// For Level(N) where N <= nesting_depth: returns domain at that level
    /// For Level(N) where N > nesting_depth: returns domain at nesting_depth (global)
    ///
    /// # Arguments
    /// * `channel` - Channel name (e.g., "x", "y")
    ///
    /// # Returns
    /// The shared domain if available and level > 0, None otherwise
    pub fn get_domain_for_channel(&self, channel: &str) -> Option<&crate::scales::DomainExtent> {
        let level = self.get_channel_level(channel);
        if level == 0 {
            return None;
        }

        // Use grammar-based lookup when partition list is available
        if self.partition_list.is_some() {
            return self.get_domain_for_channel_grammar(channel, level);
        }

        // Legacy path: Level(N) means "share across N levels of facet hierarchy from innermost"
        // Higher Level = more global sharing (Level(1) = local, Level(max) = global)
        //
        // Domain storage:
        // - depth 1 = global domain (computed at outermost facet)
        // - depth 2 = per-outer-cell domain (computed at second facet level)
        // - etc.
        //
        // Formula: target_depth = nesting_depth - level + 2
        // - Level(N) at nesting_depth D looks up depth (D - N + 2)
        // - Clamped to [1, nesting_depth] for valid range
        //
        // Note: The +2 compensates for nesting_depth typically being 1 less than
        // the actual depth hierarchy due to how coordination context is propagated.
        //
        // Examples with nesting_depth=1 (2-level facet structure):
        // - Level(1) → depth = 1-1+2 = 2, clamped down triggers fallback to per_cell_depth
        // - Level(2) → depth = 1-2+2 = 1 (global)
        if self.nesting_depth == 0 {
            return None;
        }

        let target_depth = (self.nesting_depth as i32) - (level as i32) + 2;
        let clamped_depth = target_depth.clamp(1, self.nesting_depth as i32) as usize;

        // Only use per_cell_depth fallback when target_depth was clamped DOWN from above.
        // This handles 3-level structures where Level(1) wants per-parent domains (depth=2)
        // but the formula gives target_depth=2 which gets clamped to nesting_depth=1.
        // In that case, per_cell_depth=2 might have the domains we want.
        //
        // We must NOT use this fallback when target_depth <= nesting_depth (no clamping occurred)
        // because that would cause 4-level Level(2) to incorrectly prefer depth=3 over depth=2.
        let target_was_clamped_down = target_depth > self.nesting_depth as i32;
        if target_was_clamped_down {
            let per_cell_depth = self.nesting_depth + 1;
            let per_cell_key = LevelChannelKey::new(per_cell_depth, channel);
            if let Some(domain) = self.level_domains.get(&per_cell_key) {
                return Some(domain);
            }
        }

        // Use formula-computed depth
        let key = LevelChannelKey::new(clamped_depth, channel);
        self.level_domains.get(&key)
    }

    /// Get domain for a channel using the grammar-based formula.
    ///
    /// Uses the formula: `storage_depth = partition_depth + 1` where:
    /// - partition_depth = max(0, grammar_depth - level)
    /// - grammar_depth = len(partition_list)
    ///
    /// Storage convention:
    /// - depth=1: Global domain (all data)
    /// - depth=2: Per-parent domain (per outer facet cell)
    /// - depth=N: Per (N-1) ancestor domain
    ///
    /// Level(N) semantics:
    /// - Level(1): Share with siblings (same parent) → storage_depth = depth
    /// - Level(2): Share with cousins (same grandparent) → storage_depth = depth - 1
    /// - Level(∞): Global sharing → storage_depth = 1
    fn get_domain_for_channel_grammar(
        &self,
        channel: &str,
        level: u8,
    ) -> Option<&crate::scales::DomainExtent> {
        // Grammar formula: partition_depth = max(0, depth - level)
        let partition_depth = self.partition_depth(level);

        // Convert to storage convention: storage uses 1-based depths
        // - partition_depth=0 (global) → storage_depth=1
        // - partition_depth=1 (per-parent) → storage_depth=2
        let storage_depth = partition_depth + 1;

        let key = LevelChannelKey::new(storage_depth, channel);
        self.level_domains.get(&key)
    }

    /// Get the shared domain for a Level(N) channel with explicitly provided level
    ///
    /// Unlike `get_domain_for_channel`, this method accepts the level as a parameter
    /// instead of looking it up from `channel_sharing_levels`. This is useful when
    /// the level is known from `scale_sharing_by_channel` but not stored in the
    /// coordination context.
    ///
    /// # Arguments
    /// * `channel` - Channel name (e.g., "x", "y")
    /// * `level` - The sharing level (1..254)
    ///
    /// # Returns
    /// The shared domain if available, None otherwise
    pub fn get_domain_for_channel_with_level(
        &self,
        channel: &str,
        level: u8,
    ) -> Option<&crate::scales::DomainExtent> {
        if level == 0 || level == 255 {
            return None;
        }

        // Use grammar-based lookup when partition list is available
        if self.partition_list.is_some() {
            return self.get_domain_for_channel_grammar(channel, level);
        }

        // Legacy path: For single-level facets (nesting_depth=0), Level(N) shares across all cells.
        // The domain is stored at per_cell_depth = nesting_depth + 1 = 1.
        if self.nesting_depth == 0 {
            let per_cell_depth = 1;
            let per_cell_key = LevelChannelKey::new(per_cell_depth, channel);
            return self.level_domains.get(&per_cell_key);
        }

        let target_depth = (self.nesting_depth as i32) - (level as i32) + 2;
        let clamped_depth = target_depth.clamp(1, self.nesting_depth as i32) as usize;

        // Only use per_cell_depth fallback when target_depth was clamped DOWN from above
        let target_was_clamped_down = target_depth > self.nesting_depth as i32;
        if target_was_clamped_down {
            let per_cell_depth = self.nesting_depth + 1;
            let per_cell_key = LevelChannelKey::new(per_cell_depth, channel);
            if let Some(domain) = self.level_domains.get(&per_cell_key) {
                return Some(domain);
            }
        }

        // Use formula-computed depth
        let key = LevelChannelKey::new(clamped_depth, channel);
        self.level_domains.get(&key)
    }

    /// Check if this position is at the edge for a given hierarchy level
    ///
    /// Uses position_path and level_counts to determine if the current position
    /// is at the edge (last position) at the specified level in the hierarchy.
    ///
    /// # Arguments
    /// * `level` - The hierarchy level to check (0 = innermost, higher = outer)
    ///
    /// # Returns
    /// True if at edge for this level, false otherwise
    #[cfg(test)]
    pub fn is_at_edge_for_level(&self, level: usize) -> bool {
        if level >= self.position_path.len() || level >= self.level_counts.len() {
            // Level doesn't exist in hierarchy, consider it "at edge" by default
            return true;
        }

        // Index from the end since position_path/level_counts are ordered outer->inner
        // Actually, based on docs: [outer_pos, middle_pos, inner_pos]
        // and [outer_count, middle_count, inner_count]
        // So level 0 means checking the LAST element (innermost)
        // Level 1 means checking the second-to-last element, etc.

        // For level N, we need to check position_path[depth - level - 1] vs level_counts[depth - level - 1]
        // where depth = position_path.len()
        let depth = self.position_path.len();
        if level >= depth {
            return true;
        }

        let index = depth - 1 - level;
        let position = self.position_path[index];
        let count = self.level_counts[index];

        // Edge is the last position
        count > 0 && position == count - 1
    }

    /// Builder: Set nesting depth
    ///
    /// The nesting depth indicates how deep in the facet hierarchy this facet is.
    /// 0 = outermost (root), 1 = one level nested, etc.
    pub fn with_nesting_depth(mut self, depth: usize) -> Self {
        self.nesting_depth = depth;
        self
    }

    /// Builder: Set channel sharing levels
    ///
    /// Maps channel names to their sharing level values (0-255).
    /// Typically computed from ScaleSharing::to_level().
    pub fn with_channel_sharing_levels(mut self, levels: HashMap<String, u8>) -> Self {
        self.channel_sharing_levels = levels;
        self
    }

    /// Builder: Set level-based domains
    ///
    /// Maps (level, channel) pairs to pre-computed data extents.
    /// Uses IndexMap to preserve insertion order for deterministic serialization.
    pub fn with_level_domains(
        mut self,
        domains: IndexMap<LevelChannelKey, crate::scales::DomainExtent>,
    ) -> Self {
        self.level_domains = domains;
        self
    }

    /// Builder: Set position path
    ///
    /// The position path tracks the position through each level of the hierarchy.
    /// For example, [0, 2, 1] means position 0 at outermost, 2 at middle, 1 at innermost.
    pub fn with_position_path(mut self, path: Vec<usize>) -> Self {
        self.position_path = path;
        self
    }

    /// Builder: Set level counts
    ///
    /// The count at each level of the hierarchy.
    /// For example, [3, 4, 2] means 3 subplots at outermost, 4 at middle, 2 at innermost.
    pub fn with_level_counts(mut self, counts: Vec<usize>) -> Self {
        self.level_counts = counts;
        self
    }

    /// Builder: Set explicit partition list for grammar-based visibility.
    ///
    /// When set, this enables structure-derived visibility decisions rather than
    /// incremental edge tracking. The partition list captures the complete facet
    /// hierarchy with domain values computed upfront.
    pub fn with_partition_list(
        mut self,
        partition_list: crate::facet::partition::FacetPartitionList,
    ) -> Self {
        self.partition_list = Some(partition_list);
        self
    }

    /// Builder: Set visibility cache
    ///
    /// The visibility cache pre-computes visibility decisions for all subplots
    /// based on the grammar's rules, rather than computing incrementally.
    pub fn with_visibility_cache(
        mut self,
        cache: crate::facet::partition::VisibilityCache,
    ) -> Self {
        self.visibility_cache = Some(cache);
        self
    }

    /// Compute and set visibility cache from partition_list
    ///
    /// This method computes visibility decisions for all subplots based on the
    /// grammar's rules. It requires a partition_list to be set first.
    ///
    /// # Arguments
    /// * `x_sharing` - Scale sharing level for x-axis (0=Free, 255=Shared)
    /// * `y_sharing` - Scale sharing level for y-axis (0=Free, 255=Shared)
    /// * `x_position` - X-axis position (typically Bottom)
    /// * `y_position` - Y-axis position (typically Left)
    pub fn compute_visibility_cache(
        mut self,
        x_sharing: u8,
        y_sharing: u8,
        x_position: crate::facet::context::AxisPosition,
        y_position: crate::facet::context::AxisPosition,
    ) -> Self {
        if let Some(ref partition_list) = self.partition_list {
            let cache = crate::facet::partition::VisibilityCache::compute(
                partition_list,
                x_sharing,
                y_sharing,
                x_position,
                y_position,
            );
            self.visibility_cache = Some(cache);
        }
        self
    }

    /// Get visibility for a subplot index
    ///
    /// Returns the pre-computed visibility decisions if available.
    pub fn get_visibility(
        &self,
        subplot: &crate::facet::partition::SubplotIndex,
    ) -> Option<&crate::facet::partition::SubplotVisibility> {
        self.visibility_cache
            .as_ref()
            .and_then(|cache| cache.get(subplot))
    }

    /// Upgrade from legacy coordination context
    ///
    /// This method populates the level-based fields from legacy fields
    /// for backward compatibility. It converts:
    /// - shared_data_extents -> level_domains (at level 1)
    /// - outer_position/outer_count -> position_path/level_counts (single level)
    ///
    /// This should be called when receiving a coordination context that may
    /// have been created by older code.
    pub fn upgrade_from_legacy(&mut self) {
        // Convert shared_data_extents to level_domains at level 1
        if self.level_domains.is_empty() {
            if let Some(ref extents) = self.shared_data_extents {
                for (channel, extent) in extents {
                    let key = LevelChannelKey::new(1, channel);
                    self.level_domains.insert(key, extent.clone());
                }
            }
        }

        // Convert outer_position/outer_count to position_path/level_counts
        if self.position_path.is_empty() && self.outer_count > 0 {
            self.position_path = vec![self.outer_position];
            self.level_counts = vec![self.outer_count];
            self.nesting_depth = 1;
        }
    }

    /// Create an upgraded copy of this context
    ///
    /// Like upgrade_from_legacy() but returns a new context instead of
    /// modifying in place.
    pub fn upgraded_from_legacy(&self) -> Self {
        let mut ctx = self.clone();
        ctx.upgrade_from_legacy();
        ctx
    }

    // ========================================================================
    // Debug Assertions
    // ========================================================================

    /// Validate context consistency invariants (debug builds only)
    ///
    /// This function checks critical invariants that must hold for correct operation:
    /// - inner_channel must be "row" or "col" when present
    /// - position_path length should be consistent with nesting_depth
    ///
    /// # Panics (debug builds only)
    /// Panics with a descriptive message if any invariant is violated.
    #[cfg(debug_assertions)]
    pub fn validate_consistency(&self, context: &str) {
        // Invariant: inner_channel must be "row" or "col" when set
        if let Some(ref channel) = self.inner_channel {
            debug_assert!(
                channel == "row" || channel == "col",
                "[FacetCoordinationContext] {context}: inner_channel must be 'row' or 'col', got '{channel}'"
            );
        }

        // Invariant: position_path length should equal nesting_depth when both are set
        // Note: This is a soft invariant - some code paths may not set position_path
        if !self.position_path.is_empty() && self.nesting_depth > 0 {
            // position_path may be shorter during construction
            debug_assert!(
                self.position_path.len() <= self.nesting_depth + 1,
                "[FacetCoordinationContext] {context}: position_path.len() ({}) should not exceed nesting_depth + 1 ({})",
                self.position_path.len(),
                self.nesting_depth + 1
            );
        }

        // Invariant: level_counts should match position_path length when both are set
        if !self.position_path.is_empty() && !self.level_counts.is_empty() {
            debug_assert!(
                self.position_path.len() == self.level_counts.len(),
                "[FacetCoordinationContext] {context}: position_path.len() ({}) must equal level_counts.len() ({})",
                self.position_path.len(),
                self.level_counts.len()
            );
        }

        // Invariant: position < count for each level in position_path/level_counts
        for (i, (pos, count)) in self
            .position_path
            .iter()
            .zip(self.level_counts.iter())
            .enumerate()
        {
            debug_assert!(
                *pos < *count || *count == 0,
                "[FacetCoordinationContext] {context}: position[{i}] ({pos}) must be < count[{i}] ({count})"
            );
        }

        tracing::trace!(
            "[FacetCoordinationContext] {context}: consistency check passed (nesting_depth={}, position_path.len()={})",
            self.nesting_depth,
            self.position_path.len()
        );
    }

    /// No-op for release builds
    #[cfg(not(debug_assertions))]
    #[inline]
    pub fn validate_consistency(&self, _context: &str) {
        // Intentionally empty - validation only runs in debug builds
    }
}

/// Log an invariant check result (debug builds only)
///
/// This helper provides structured logging for invariant checks, making it easier
/// to trace invariant validation during debugging.
#[cfg(debug_assertions)]
pub fn log_invariant_check(name: &str, passed: bool, context: &str) {
    if passed {
        tracing::trace!("[Invariant] {name}: PASSED ({context})");
    } else {
        tracing::warn!("[Invariant] {name}: FAILED ({context})");
    }
}

/// No-op for release builds
#[cfg(not(debug_assertions))]
#[inline]
pub fn log_invariant_check(_name: &str, _passed: bool, _context: &str) {
    // Intentionally empty - logging only in debug builds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_tree() -> Arc<EvaluatedFacetTree> {
        Arc::new(EvaluatedFacetTree::empty())
    }

    #[test]
    fn test_guide_suppression() {
        let ctx_suppress = FacetCoordinationContext::new(
            empty_tree(),
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
            empty_tree(),
            "row",
            ScaleSharing::Shared,
            GuideOwnership::Edge,
            2,
            3,
            3,
        );
        assert!(!ctx_edge.should_suppress_guides());
        assert!(ctx_edge.should_render_guides());

        let ctx_full = FacetCoordinationContext::new(
            empty_tree(),
            "row",
            ScaleSharing::Free,
            GuideOwnership::Full,
            0,
            3,
            3,
        );
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

        // Test UInt64 (previously overflowed when stored as i64)
        let large_uint = u64::MAX;
        let u = SerializableDomainValue::from_scalar(&ScalarValue::UInt64(Some(large_uint)));
        assert!(matches!(u, SerializableDomainValue::UInt64(v) if v == large_uint));
        assert_eq!(u.to_scalar(), ScalarValue::UInt64(Some(large_uint)));

        // Test Decimal128
        let d = SerializableDomainValue::from_scalar(&ScalarValue::Decimal128(Some(12345), 10, 2));
        assert!(matches!(d, SerializableDomainValue::Decimal128(_)));
        assert_eq!(d.to_scalar(), ScalarValue::Decimal128(Some(12345), 10, 2));

        // Test timestamps
        let ts_ms = SerializableDomainValue::from_scalar(&ScalarValue::TimestampMillisecond(
            Some(1609459200000),
            None,
        ));
        assert!(matches!(
            ts_ms,
            SerializableDomainValue::TimestampMs(1609459200000)
        ));
        assert_eq!(
            ts_ms.to_scalar(),
            ScalarValue::TimestampMillisecond(Some(1609459200000), None)
        );

        let ts_us = SerializableDomainValue::from_scalar(&ScalarValue::TimestampMicrosecond(
            Some(1609459200000000),
            None,
        ));
        assert!(matches!(
            ts_us,
            SerializableDomainValue::TimestampUs(1609459200000000)
        ));

        let ts_ns = SerializableDomainValue::from_scalar(&ScalarValue::TimestampNanosecond(
            Some(1609459200000000000),
            None,
        ));
        assert!(matches!(
            ts_ns,
            SerializableDomainValue::TimestampNs(1609459200000000000)
        ));

        // Test null
        let n = SerializableDomainValue::from_scalar(&ScalarValue::Null);
        assert!(matches!(n, SerializableDomainValue::Null));

        // Test Dictionary - should unwrap to underlying value
        use datafusion::arrow::datatypes::DataType;
        let dict_value = ScalarValue::Dictionary(
            Box::new(DataType::Int32),
            Box::new(ScalarValue::Utf8(Some("category".to_string()))),
        );
        let d = SerializableDomainValue::from_scalar(&dict_value);
        assert!(matches!(d, SerializableDomainValue::String(ref s) if s == "category"));
        assert_eq!(
            d.to_scalar(),
            ScalarValue::Utf8(Some("category".to_string()))
        );

        // Test nested Dictionary (Dictionary containing Dictionary)
        let nested_dict = ScalarValue::Dictionary(
            Box::new(DataType::Int32),
            Box::new(ScalarValue::Dictionary(
                Box::new(DataType::Int8),
                Box::new(ScalarValue::Int64(Some(42))),
            )),
        );
        let nd = SerializableDomainValue::from_scalar(&nested_dict);
        assert!(matches!(nd, SerializableDomainValue::Int(42)));
    }

    // ========================================================================
    // Level-based scale sharing tests (Phase 2)
    // ========================================================================

    #[test]
    fn test_level_channel_key_equality_and_hashing() {
        use std::collections::HashSet;

        // Test equality
        let key1 = LevelChannelKey::new(1, "x");
        let key2 = LevelChannelKey::new(1, "x");
        let key3 = LevelChannelKey::new(2, "x");
        let key4 = LevelChannelKey::new(1, "y");

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_ne!(key1, key4);

        // Test hashing (can be used in HashSet)
        let mut set = HashSet::new();
        set.insert(key1.clone());
        assert!(set.contains(&key2));
        assert!(!set.contains(&key3));

        // Test as HashMap key
        let mut map = HashMap::new();
        map.insert(key1.clone(), SerializableDataExtents::interval(0.0, 100.0));
        assert!(map.get(&key2).is_some());
        assert!(map.get(&key3).is_none());
    }

    #[test]
    fn test_level_channel_key_serialization() {
        let key = LevelChannelKey::new(2, "color");

        // Serialize
        let json = serde_json::to_string(&key).unwrap();
        assert!(json.contains("\"level\":2"));
        assert!(json.contains("\"channel\":\"color\""));

        // Deserialize
        let restored: LevelChannelKey = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.level, 2);
        assert_eq!(restored.channel, "color");
    }

    #[test]
    fn test_get_channel_level_with_explicit_levels() {
        let mut levels = HashMap::new();
        levels.insert("x".to_string(), 0u8);
        levels.insert("y".to_string(), 1u8);
        levels.insert("color".to_string(), 255u8);

        let ctx = FacetCoordinationContext::default().with_channel_sharing_levels(levels);

        // Explicit levels
        assert_eq!(ctx.get_channel_level("x"), 0);
        assert_eq!(ctx.get_channel_level("y"), 1);
        assert_eq!(ctx.get_channel_level("color"), 255);

        // Default for unset channels is 0 (Free)
        assert_eq!(ctx.get_channel_level("size"), 0);
        assert_eq!(ctx.get_channel_level("unknown"), 0);
    }

    #[test]
    fn test_should_use_parent_domain_boundary_conditions() {
        // Level 0 should NOT use parent domain
        let mut levels0 = HashMap::new();
        levels0.insert("x".to_string(), 0u8);
        let ctx0 = FacetCoordinationContext::default().with_channel_sharing_levels(levels0);
        assert!(!ctx0.should_use_parent_domain("x"));

        // Level 1 SHOULD use parent domain
        let mut levels1 = HashMap::new();
        levels1.insert("x".to_string(), 1u8);
        let ctx1 = FacetCoordinationContext::default().with_channel_sharing_levels(levels1);
        assert!(ctx1.should_use_parent_domain("x"));

        // Level 255 (Shared) SHOULD use parent domain
        let mut levels255 = HashMap::new();
        levels255.insert("x".to_string(), 255u8);
        let ctx255 = FacetCoordinationContext::default().with_channel_sharing_levels(levels255);
        assert!(ctx255.should_use_parent_domain("x"));

        // Unset channel defaults to 0 (Free), NOT use parent domain
        let ctx_empty = FacetCoordinationContext::default();
        assert!(!ctx_empty.should_use_parent_domain("x"));
    }

    #[test]
    fn test_get_domain_for_channel_with_populated_domains() {
        // Set up sharing levels
        // With formula: target_depth = nesting_depth - level + 2
        // At nesting_depth=2:
        // - Level(1) → depth = 2-1+2 = 3, clamped to 2
        // - Level(2) → depth = 2-2+2 = 2
        // - Level(3) → depth = 2-3+2 = 1 (global)
        let mut levels = HashMap::new();
        levels.insert("x".to_string(), 0u8); // Free - no domain lookup
        levels.insert("y".to_string(), 2u8); // Level(2) - look up at depth 2
        levels.insert("color".to_string(), 3u8); // Level(3) - look up at depth 1 (global)

        // Set up level domains (using IndexMap for deterministic iteration)
        use crate::scales::DomainBounds;
        let mut domains = IndexMap::new();
        domains.insert(
            LevelChannelKey::new(1, "color"),
            SerializableDataExtents::discrete(vec![
                ScalarValue::Utf8(Some("red".to_string())),
                ScalarValue::Utf8(Some("blue".to_string())),
            ])
            .into(),
        );
        domains.insert(
            LevelChannelKey::new(2, "y"),
            SerializableDataExtents::interval(0.0, 100.0).into(),
        );

        let ctx = FacetCoordinationContext::default()
            .with_nesting_depth(2)
            .with_channel_sharing_levels(levels)
            .with_level_domains(domains);

        // Free channel (x) returns None
        assert!(ctx.get_domain_for_channel("x").is_none());

        // Level(2) channel (y) returns the domain at depth 2
        let y_domain = ctx.get_domain_for_channel("y");
        assert!(y_domain.is_some());
        match &y_domain.unwrap().bounds {
            DomainBounds::Numeric { min, max } => {
                assert_eq!(*min, 0.0);
                assert_eq!(*max, 100.0);
            }
            _ => panic!("Expected Numeric"),
        }

        // Level(3) channel (color) returns the domain at depth 1 (global)
        let color_domain = ctx.get_domain_for_channel("color");
        assert!(color_domain.is_some());
        match &color_domain.unwrap().bounds {
            DomainBounds::Discrete(values) => {
                assert_eq!(values.len(), 2);
            }
            _ => panic!("Expected Discrete"),
        }
    }

    #[test]
    fn test_get_domain_for_channel_with_empty_domains() {
        let mut levels = HashMap::new();
        levels.insert("y".to_string(), 1u8);

        // No domains set, but level_domains is empty IndexMap
        let ctx = FacetCoordinationContext::default()
            .with_nesting_depth(1)
            .with_channel_sharing_levels(levels);

        // Should return None when domain not found
        assert!(ctx.get_domain_for_channel("y").is_none());
    }

    #[test]
    fn test_get_domain_for_channel_level_clamping() {
        // Set up a channel with Level(5) but nesting_depth is only 2
        // With formula: target_depth = nesting_depth - level + 2 = 2 - 5 + 2 = -1
        // Clamped to 1 (global level)
        let mut levels = HashMap::new();
        levels.insert("y".to_string(), 5u8);

        // Domain at level 1 (global, where high Level values clamp to)
        use crate::scales::DomainBounds;
        let mut domains = IndexMap::new();
        domains.insert(
            LevelChannelKey::new(1, "y"),
            SerializableDataExtents::interval(0.0, 100.0).into(),
        );

        let ctx = FacetCoordinationContext::default()
            .with_nesting_depth(2)
            .with_channel_sharing_levels(levels)
            .with_level_domains(domains);

        // Should clamp to depth 1 (global) for high Level values
        let domain = ctx.get_domain_for_channel("y");
        assert!(domain.is_some());
        match &domain.unwrap().bounds {
            DomainBounds::Numeric { min, max } => {
                assert_eq!(*min, 0.0);
                assert_eq!(*max, 100.0);
            }
            _ => panic!("Expected Numeric"),
        }
    }

    #[test]
    fn test_get_domain_for_channel_at_outermost() {
        // When nesting_depth is 0, there's nothing to share with
        let mut levels = HashMap::new();
        levels.insert("y".to_string(), 1u8);

        let ctx = FacetCoordinationContext::default()
            .with_nesting_depth(0)
            .with_channel_sharing_levels(levels);

        // Should return None at outermost (no parent to share with)
        assert!(ctx.get_domain_for_channel("y").is_none());
    }

    #[test]
    fn test_is_at_edge_for_level_various_combinations() {
        // 3-level hierarchy: [outer_pos, middle_pos, inner_pos]
        // Position [0, 2, 1] in counts [3, 4, 2]
        let ctx = FacetCoordinationContext::default()
            .with_position_path(vec![0, 2, 1])
            .with_level_counts(vec![3, 4, 2]);

        // Level 0 = innermost: position 1 in count 2 -> at edge (last)
        assert!(ctx.is_at_edge_for_level(0));

        // Level 1 = middle: position 2 in count 4 -> NOT at edge
        assert!(!ctx.is_at_edge_for_level(1));

        // Level 2 = outer: position 0 in count 3 -> NOT at edge
        assert!(!ctx.is_at_edge_for_level(2));

        // Level 3+ beyond hierarchy -> true by default
        assert!(ctx.is_at_edge_for_level(3));
        assert!(ctx.is_at_edge_for_level(100));
    }

    #[test]
    fn test_is_at_edge_for_level_all_at_edge() {
        // All positions are at their respective edges (last positions)
        let ctx = FacetCoordinationContext::default()
            .with_position_path(vec![2, 3, 1])
            .with_level_counts(vec![3, 4, 2]);

        assert!(ctx.is_at_edge_for_level(0)); // 1 == 2-1
        assert!(ctx.is_at_edge_for_level(1)); // 3 == 4-1
        assert!(ctx.is_at_edge_for_level(2)); // 2 == 3-1
    }

    #[test]
    fn test_is_at_edge_for_level_empty_hierarchy() {
        // Empty position_path and level_counts
        let ctx = FacetCoordinationContext::default();

        // Any level should be considered at edge when hierarchy is empty
        assert!(ctx.is_at_edge_for_level(0));
        assert!(ctx.is_at_edge_for_level(1));
    }

    #[test]
    fn test_builder_pattern_chaining() {
        let mut levels = HashMap::new();
        levels.insert("x".to_string(), 0u8);
        levels.insert("y".to_string(), 1u8);

        let mut domains = IndexMap::new();
        domains.insert(
            LevelChannelKey::new(1, "y"),
            SerializableDataExtents::interval(0.0, 100.0).into(),
        );

        let ctx = FacetCoordinationContext::default()
            .with_nesting_depth(2)
            .with_channel_sharing_levels(levels)
            .with_level_domains(domains)
            .with_position_path(vec![0, 1, 2])
            .with_level_counts(vec![3, 4, 5]);

        // Verify all fields were set
        assert_eq!(ctx.nesting_depth, 2);
        assert_eq!(ctx.channel_sharing_levels.len(), 2);
        assert_eq!(ctx.level_domains.len(), 1);
        assert_eq!(ctx.position_path, vec![0, 1, 2]);
        assert_eq!(ctx.level_counts, vec![3, 4, 5]);
    }

    #[test]
    fn test_upgrade_from_legacy() {
        // Create a legacy-style context (using old fields)
        let mut shared_extents = HashMap::new();
        shared_extents.insert(
            "y".to_string(),
            SerializableDataExtents::interval(0.0, 100.0).into(),
        );

        let mut ctx = FacetCoordinationContext::new(
            empty_tree(),
            "row",
            ScaleSharing::Shared,
            GuideOwnership::Edge,
            2, // outer_position
            5, // outer_count
            3,
        )
        .with_shared_data_extents(shared_extents);

        // Before upgrade, level fields should be empty/default
        assert!(ctx.level_domains.is_empty());
        assert!(ctx.position_path.is_empty());

        // Upgrade
        ctx.upgrade_from_legacy();

        // After upgrade, level fields should be populated from shared_data_extents
        assert_eq!(ctx.level_domains.len(), 1);
        let y_key = LevelChannelKey::new(1, "y");
        assert!(ctx.level_domains.contains_key(&y_key));

        // position_path and level_counts from outer_position/outer_count
        assert_eq!(ctx.position_path, vec![2]);
        assert_eq!(ctx.level_counts, vec![5]);
        assert_eq!(ctx.nesting_depth, 1);
    }

    #[test]
    fn test_upgraded_from_legacy_preserves_original() {
        let original = FacetCoordinationContext::default().with_outer_position(1, 3);

        // Create upgraded copy
        let upgraded = original.upgraded_from_legacy();

        // Original should be unchanged
        assert!(original.position_path.is_empty());

        // Upgraded should have converted fields from outer_position/outer_count
        assert_eq!(upgraded.position_path, vec![1]);
        assert_eq!(upgraded.level_counts, vec![3]);
    }

    // ========================================================================
    // Uniform Free scaling tests
    // ========================================================================

    #[test]
    fn test_get_uniform_cell_count_disabled() {
        // When enable_uniform_free_scaling is false, should return None
        // even if max_inner_cell_count is set
        let ctx = FacetCoordinationContext::default()
            .with_max_inner_cell_count(3)
            .with_uniform_free_scaling(false);

        assert_eq!(ctx.max_inner_cell_count, Some(3));
        assert!(!ctx.enable_uniform_free_scaling);
        assert_eq!(ctx.get_uniform_cell_count(), None);
    }

    #[test]
    fn test_get_uniform_cell_count_enabled() {
        // When both flag is true and count is set, should return Some(count)
        let ctx = FacetCoordinationContext::default()
            .with_max_inner_cell_count(4)
            .with_uniform_free_scaling(true);

        assert_eq!(ctx.max_inner_cell_count, Some(4));
        assert!(ctx.enable_uniform_free_scaling);
        assert_eq!(ctx.get_uniform_cell_count(), Some(4));
    }

    #[test]
    fn test_get_uniform_cell_count_enabled_but_count_not_set() {
        // When flag is true but count was never set, should return None
        let ctx = FacetCoordinationContext::default().with_uniform_free_scaling(true);

        assert!(ctx.max_inner_cell_count.is_none());
        assert!(ctx.enable_uniform_free_scaling);
        assert_eq!(ctx.get_uniform_cell_count(), None);
    }

    #[test]
    fn test_uniform_cell_count_floor_at_one() {
        // count=0 should be floored to 1
        let ctx0 = FacetCoordinationContext::default()
            .with_max_inner_cell_count(0)
            .with_uniform_free_scaling(true);
        assert_eq!(ctx0.max_inner_cell_count, Some(1));
        assert_eq!(ctx0.get_uniform_cell_count(), Some(1));

        // count=1 should remain 1
        let ctx1 = FacetCoordinationContext::default()
            .with_max_inner_cell_count(1)
            .with_uniform_free_scaling(true);
        assert_eq!(ctx1.max_inner_cell_count, Some(1));
        assert_eq!(ctx1.get_uniform_cell_count(), Some(1));

        // count=2 should remain 2
        let ctx2 = FacetCoordinationContext::default()
            .with_max_inner_cell_count(2)
            .with_uniform_free_scaling(true);
        assert_eq!(ctx2.max_inner_cell_count, Some(2));
        assert_eq!(ctx2.get_uniform_cell_count(), Some(2));
    }

    #[test]
    fn test_uniform_cell_count_default_values() {
        // Default context should have no uniform scaling enabled
        let ctx = FacetCoordinationContext::default();
        assert!(ctx.max_inner_cell_count.is_none());
        assert!(!ctx.enable_uniform_free_scaling);
        assert_eq!(ctx.get_uniform_cell_count(), None);
    }

    #[test]
    fn test_serializable_domain_value_round_trip_all_variants() {
        // Test round-trip for all SerializableDomainValue variants
        let test_cases: Vec<(ScalarValue, &str)> = vec![
            (ScalarValue::Utf8(Some("test string".to_string())), "String"),
            (ScalarValue::Int64(Some(-42)), "Int (negative)"),
            (ScalarValue::Int64(Some(0)), "Int (zero)"),
            (ScalarValue::Int64(Some(i64::MAX)), "Int (max)"),
            (ScalarValue::Int64(Some(i64::MIN)), "Int (min)"),
            (ScalarValue::UInt64(Some(u64::MAX)), "UInt64 (max)"),
            (ScalarValue::UInt64(Some(0)), "UInt64 (zero)"),
            (ScalarValue::Float64(Some(3.14159)), "Float (positive)"),
            (ScalarValue::Float64(Some(-2.71828)), "Float (negative)"),
            (ScalarValue::Float64(Some(0.0)), "Float (zero)"),
            (ScalarValue::Boolean(Some(true)), "Bool (true)"),
            (ScalarValue::Boolean(Some(false)), "Bool (false)"),
            (ScalarValue::Decimal128(Some(12345), 10, 2), "Decimal128"),
            (
                ScalarValue::Decimal128(Some(-98765), 15, 4),
                "Decimal128 (negative)",
            ),
            (
                ScalarValue::TimestampMillisecond(Some(1609459200000), None),
                "TimestampMs",
            ),
            (
                ScalarValue::TimestampMicrosecond(Some(1609459200000000), None),
                "TimestampUs",
            ),
            (
                ScalarValue::TimestampNanosecond(Some(1609459200000000000), None),
                "TimestampNs",
            ),
            (ScalarValue::Null, "Null"),
        ];

        for (scalar, variant_name) in test_cases {
            // Convert to SerializableDomainValue
            let serializable = SerializableDomainValue::from_scalar(&scalar);

            // Serialize to JSON
            let json = serde_json::to_string(&serializable)
                .unwrap_or_else(|e| panic!("Failed to serialize {}: {}", variant_name, e));

            // Deserialize from JSON
            let deserialized: SerializableDomainValue = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("Failed to deserialize {}: {}", variant_name, e));

            // Convert back to ScalarValue
            let round_tripped = deserialized.to_scalar();

            // Verify equality
            // Note: Float comparison needs special handling for NaN, but we don't test NaN here
            assert_eq!(
                serializable, deserialized,
                "{}: SerializableDomainValue should match after JSON round-trip",
                variant_name
            );

            // For most types, the round-tripped ScalarValue should match
            // (except for type widening like Int8 -> Int64)
            match &scalar {
                ScalarValue::Null => assert_eq!(round_tripped, ScalarValue::Null),
                ScalarValue::Boolean(Some(b)) => {
                    assert_eq!(round_tripped, ScalarValue::Boolean(Some(*b)))
                }
                ScalarValue::Utf8(Some(s)) => {
                    assert_eq!(round_tripped, ScalarValue::Utf8(Some(s.clone())))
                }
                ScalarValue::Int64(Some(n)) => {
                    assert_eq!(round_tripped, ScalarValue::Int64(Some(*n)))
                }
                ScalarValue::UInt64(Some(n)) => {
                    assert_eq!(round_tripped, ScalarValue::UInt64(Some(*n)))
                }
                ScalarValue::Float64(Some(f)) => {
                    // Float comparison with tolerance
                    if let ScalarValue::Float64(Some(f2)) = round_tripped {
                        assert!((f - f2).abs() < 1e-10, "Float mismatch: {} vs {}", f, f2);
                    } else {
                        panic!("Expected Float64, got {:?}", round_tripped);
                    }
                }
                ScalarValue::Decimal128(Some(v), p, s) => {
                    assert_eq!(round_tripped, ScalarValue::Decimal128(Some(*v), *p, *s))
                }
                ScalarValue::TimestampMillisecond(Some(ts), _) => {
                    assert_eq!(
                        round_tripped,
                        ScalarValue::TimestampMillisecond(Some(*ts), None)
                    )
                }
                ScalarValue::TimestampMicrosecond(Some(ts), _) => {
                    assert_eq!(
                        round_tripped,
                        ScalarValue::TimestampMicrosecond(Some(*ts), None)
                    )
                }
                ScalarValue::TimestampNanosecond(Some(ts), _) => {
                    assert_eq!(
                        round_tripped,
                        ScalarValue::TimestampNanosecond(Some(*ts), None)
                    )
                }
                _ => {} // Other types may have intentional type changes
            }
        }
    }

    // ========================================================================
    // Accessor View Tests
    // ========================================================================

    #[test]
    fn test_guide_context_view() {
        let ctx = FacetCoordinationContext::new(
            empty_tree(),
            "row",
            ScaleSharing::Shared,
            GuideOwnership::Edge,
            2, // outer_position
            3, // outer_count
            3,
        );

        let view = ctx.guide_view();
        assert_eq!(view.ownership, GuideOwnership::Edge);
        assert_eq!(view.outer_position, 2);
        assert_eq!(view.outer_count, 3);
        assert!(view.should_render());
        assert!(!view.should_suppress());
        assert!(view.is_edge_position()); // position 2 of 3 is the last (edge)

        // Test non-edge position
        let ctx_interior = FacetCoordinationContext::new(
            empty_tree(),
            "row",
            ScaleSharing::Shared,
            GuideOwnership::Suppress,
            0,
            3,
            3,
        );
        let view_interior = ctx_interior.guide_view();
        assert!(!view_interior.is_edge_position());
        assert!(view_interior.should_suppress());
    }

    #[test]
    fn test_overflow_context_view() {
        let mut ctx = FacetCoordinationContext::default();
        ctx.row_overflow_by_index = Some(vec![
            OverflowSpaceRequirement::default(),
            OverflowSpaceRequirement::default(),
        ]);

        let view = ctx.overflow_view();
        assert!(view.row_overflow.is_some());
        assert!(view.col_overflow.is_none());
        assert!(view.row_at(0).is_some());
        assert!(view.row_at(5).is_none()); // Out of bounds
        assert!(view.col_at(0).is_none()); // Not set
    }

    #[test]
    fn test_validate_consistency_passes() {
        // Valid context should pass validation
        let ctx = FacetCoordinationContext::new(
            empty_tree(),
            "row",
            ScaleSharing::Shared,
            GuideOwnership::Edge,
            1,
            3,
            3,
        )
        .with_nesting_depth(1);

        // This should not panic
        ctx.validate_consistency("test");
    }

    #[test]
    fn test_validate_consistency_with_position_path() {
        let mut ctx = FacetCoordinationContext::default().with_nesting_depth(2);
        ctx.position_path = vec![0, 1];
        ctx.level_counts = vec![3, 4];
        ctx.inner_channel = Some("col".to_string());

        // This should not panic - all invariants hold
        ctx.validate_consistency("test_position_path");
    }
}
