//! Iterator abstraction for consistent subplot iteration in faceted layouts
//!
//! Ensures that every subplot iteration gets correct FacetContext with invariants enforced:
//! - position matches the iteration index (combined with parent context for nested facets)
//! - grid_dimensions matches the total number of subplots (combined with parent context for nested facets)
//! - unified_channels are computed consistently from DimConfig
//! - FacetContext is always present in params
//!
//! For nested facets, the iterator uses FacetCoordinationContext to compute proper grid positions:
//! - outer_position and outer_count from coordination context determine the parent dimension
//! - This enables correct edge detection for axis label visibility

use crate::channel::config_traits::ScaleSharing;
use crate::facet::context::AxisPosition;
use crate::facet::coordination::FacetCoordinationContext;
use crate::facet::dimension_config::FacetDimensionConfig;
use crate::facet::partition::{build_subplot_index_from_params, compute_subplot_visibility};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

/// Parameter key for additional unified channels beyond what DimConfig specifies.
/// Used for same-type nesting (e.g., FacetRow wrapping FacetRow) where the outer
/// FacetRow needs to unify "x" in addition to the default "y".
pub const ADDITIONAL_UNIFIED_CHANNELS_KEY: &str = "FACET_ADDITIONAL_UNIFIED_CHANNELS";

/// Iteration state for a single subplot in a faceted layout
#[derive(Debug, Clone)]
pub struct SubplotIteration {
    /// 0-based index of this subplot within the facet dimension
    pub index: usize,
    /// The domain value that defines this subplot (e.g., species name)
    pub facet_value: ScalarValue,
    /// Parameters with FacetContext merged in (ready to pass to subplot operations)
    pub params: IndexMap<String, ScalarValue>,
}

/// Iterator over subplots in a faceted layout (generic over row/column dimension)
///
/// Ensures each subplot gets correct FacetContext with:
/// - position matching the iteration index (parameterized by DimConfig)
/// - grid_dimensions matching the total number of subplots (parameterized by DimConfig)
/// - unified_channels automatically determined by DimConfig
///
/// # Example
///
/// ```ignore
/// use crate::facet::dimension_config::RowDimensionConfig;
///
/// let iterator = SubplotIterator::<RowDimensionConfig>::new(
///     domain_vals,
///     params.clone(),
///     scale_sharing,
/// );
///
/// for iteration in iterator {
///     // iteration.params guaranteed to have correct FacetContext
///     subplot.measure_guide_overflow_with_scales(
///         &scales,
///         width,
///         height,
///         ctx,
///         &iteration.params,  // ✓ FacetContext present and correct
///     ).await?;
/// }
/// ```
pub struct SubplotIterator<DimConfig: FacetDimensionConfig> {
    domain_vals: Vec<ScalarValue>,
    base_params: IndexMap<String, ScalarValue>,
    scale_sharing: std::collections::HashMap<String, ScaleSharing>,
    current_index: usize,
    /// Direct coordination context (preferred over extracting from params)
    coordination_context: Option<FacetCoordinationContext>,
    _phantom: std::marker::PhantomData<DimConfig>,
}

impl<DimConfig: FacetDimensionConfig> SubplotIterator<DimConfig> {
    /// Create a new subplot iterator
    ///
    /// # Arguments
    /// * `domain_vals` - The domain values to iterate over (one per subplot)
    /// * `base_params` - Base parameters to merge FacetContext into
    /// * `scale_sharing` - Per-channel scale sharing configuration (ScaleSharing enum)
    /// * `coordination_context` - Optional coordination context for nested facets (passed directly)
    ///
    /// Note: unified_channels are automatically determined from DimConfig::unified_channels()
    pub fn new(
        domain_vals: Vec<ScalarValue>,
        base_params: IndexMap<String, ScalarValue>,
        scale_sharing: std::collections::HashMap<String, ScaleSharing>,
        coordination_context: Option<FacetCoordinationContext>,
    ) -> Self {
        Self {
            domain_vals,
            base_params,
            scale_sharing,
            current_index: 0,
            coordination_context,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Get the number of subplots in this iterator
    pub fn len(&self) -> usize {
        self.domain_vals.len()
    }

    /// Check if this iterator is empty
    pub fn is_empty(&self) -> bool {
        self.domain_vals.is_empty()
    }
}

impl<DimConfig: FacetDimensionConfig> Iterator for SubplotIterator<DimConfig> {
    type Item = SubplotIteration;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_index >= self.domain_vals.len() {
            return None;
        }

        let index = self.current_index;
        let facet_value = self.domain_vals[index].clone();

        // Create FacetContext with invariants enforced, using DimConfig for position/grid/unified_channels
        // For nested facets, we need to MERGE unified_channels from parent context, not replace
        use crate::facet::context::FacetContext;

        // Get unified_channels: merge parent's with this dimension's
        let mut unified_channels =
            if let Some(parent_ctx) = FacetContext::from_params(&self.base_params) {
                // Merge parent's unified_channels with this dimension's
                let mut merged = parent_ctx.unified_channels;
                merged.extend(DimConfig::unified_channels().iter().map(|s| s.to_string()));
                merged
            } else {
                // No parent context, use just this dimension's channels
                DimConfig::unified_channels()
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            };

        // Check for additional unified channels from same-type nesting
        // (e.g., FacetRow with unified_x_title unifies both y and x)
        // We use a simpler Utf8 scalar with comma-separated channel names
        if let Some(ScalarValue::Utf8(Some(channels_str))) =
            self.base_params.get(ADDITIONAL_UNIFIED_CHANNELS_KEY)
        {
            for ch in channels_str.split(',') {
                let ch = ch.trim();
                if !ch.is_empty() {
                    unified_channels.insert(ch.to_string());
                }
            }
        }

        // Use coordination context passed directly (not from params)
        // This preserves non-serializable fields like partition_list
        let coordination_ctx = self.coordination_context.as_ref();

        // Compute position and grid_dimensions
        // For nested facets, combine inner index with outer position
        let (position, grid_dimensions) = if let Some(ref coord_ctx) = coordination_ctx {
            // Check if this coordination context is meant for this channel
            let current_channel = DimConfig::channel_name();
            if coord_ctx.inner_channel.as_deref() == Some(current_channel) {
                // This coordination is for us - use outer position to compute true grid position
                // Use uniform_cell_count (for Free scaling) or inner_domain_count (for Shared) for
                // consistent grid dimensions across all outer subplots.
                let inner_count = if let Some(uniform_count) = coord_ctx.get_uniform_cell_count() {
                    // Uniform Free scaling: use max cell count across all outer cells
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "SubplotIterator: channel={} using uniform_cell_count={} (enable_uniform={} max_inner={:?})",
                            current_channel,
                            uniform_count,
                            coord_ctx.enable_uniform_free_scaling,
                            coord_ctx.max_inner_cell_count
                        );
                    }
                    uniform_count
                } else if coord_ctx.inner_domain_count > 0 {
                    // Shared domain: use coordinated domain count
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "SubplotIterator: channel={} using inner_domain_count={} (enable_uniform={} max_inner={:?})",
                            current_channel,
                            coord_ctx.inner_domain_count,
                            coord_ctx.enable_uniform_free_scaling,
                            coord_ctx.max_inner_cell_count
                        );
                    }
                    coord_ctx.inner_domain_count
                } else {
                    // Fallback to actual domain size if neither is set
                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                        eprintln!(
                            "SubplotIterator: channel={} FALLBACK to domain_vals.len()={} (enable_uniform={} max_inner={:?})",
                            current_channel,
                            self.domain_vals.len(),
                            coord_ctx.enable_uniform_free_scaling,
                            coord_ctx.max_inner_cell_count
                        );
                    }
                    self.domain_vals.len()
                };

                // Adjust index by phantom_prepend_count to get rendered position
                // When phantoms are prepended, actual data starts at position phantom_prepend_count
                let adjusted_index = index + coord_ctx.phantom_prepend_count;

                // Check if this is same-type nesting (Row>Row or Col>Col)
                // In same-type nesting, the orthogonal dimension is always 1
                // We detect same-type by checking if the outer facet's channel matches this facet type.
                let is_same_type = coord_ctx
                    .outer_channel
                    .as_ref()
                    .map(|outer| outer == current_channel)
                    .unwrap_or(false);

                let (pos, dims) = if DimConfig::is_row_facet() {
                    // This is FacetRow inside another facet
                    let row = adjusted_index;
                    if is_same_type {
                        // Row inside Row: all rows are in column 0
                        // position: (row_index, 0)
                        // grid_dimensions: (num_rows, 1)
                        ((row, 0), (inner_count, 1))
                    } else {
                        // Row inside Column (cross-type nesting)
                        // position: (row_index, column_position_from_outer)
                        // grid_dimensions: (num_rows, num_columns_from_outer)
                        let col = coord_ctx.outer_position;
                        let num_cols = if coord_ctx.outer_count > 0 {
                            coord_ctx.outer_count
                        } else {
                            1
                        };
                        ((row, col), (inner_count, num_cols))
                    }
                } else {
                    // This is FacetColumn inside another facet
                    let col = adjusted_index;
                    if is_same_type {
                        // Col inside Col: all columns are in row 0
                        // position: (0, col_index)
                        // grid_dimensions: (1, num_cols)
                        ((0, col), (1, inner_count))
                    } else {
                        // Column inside Row (cross-type nesting)
                        // position: (row_position_from_outer, column_index)
                        // grid_dimensions: (num_rows_from_outer, num_columns)
                        let row = coord_ctx.outer_position;
                        let num_rows = if coord_ctx.outer_count > 0 {
                            coord_ctx.outer_count
                        } else {
                            1
                        };
                        ((row, col), (num_rows, inner_count))
                    }
                };
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "SubplotIterator: channel={} inner_channel={:?} is_same_type={} coord matched! outer_pos={} outer_count={} -> position=({},{}) grid=({},{})",
                        current_channel,
                        coord_ctx.inner_channel,
                        is_same_type,
                        coord_ctx.outer_position,
                        coord_ctx.outer_count,
                        pos.0,
                        pos.1,
                        dims.0,
                        dims.1
                    );
                }
                (pos, dims)
            } else {
                // Coordination context is not for us (e.g., we're the outer facet) - use default
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "SubplotIterator: channel={} inner_channel={:?} - NOT FOR US, using default",
                        current_channel, coord_ctx.inner_channel
                    );
                }
                (
                    DimConfig::index_to_position(index),
                    DimConfig::count_to_grid_dimensions(self.domain_vals.len()),
                )
            }
        } else {
            // No coordination context - use default (single-level faceting)
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "SubplotIterator: channel={} - NO coordination context",
                    DimConfig::channel_name()
                );
            }
            (
                DimConfig::index_to_position(index),
                DimConfig::count_to_grid_dimensions(self.domain_vals.len()),
            )
        };

        // Build scale_sharing: start with parent's, then overlay this facet's, then coordination context
        // This ensures that y-axis sharing from outer FacetRow is preserved when inner FacetCol iterates
        let mut scale_sharing =
            if let Some(parent_ctx) = FacetContext::from_params(&self.base_params) {
                parent_ctx.scale_sharing.clone()
            } else {
                std::collections::HashMap::new()
            };

        // Overlay this facet's scale_sharing (don't overwrite parent's values)
        for (k, v) in self.scale_sharing.iter() {
            scale_sharing.entry(k.clone()).or_insert(*v);
        }

        // Add facet channel scale sharing from coordination context if specified
        if let Some(ref coord_ctx) = coordination_ctx {
            // This enables FacetContext.should_show_facet_labels() to work correctly
            if !coord_ctx.inner_scale_sharing.is_free() {
                if let Some(ref channel) = coord_ctx.inner_channel {
                    scale_sharing.insert(channel.clone(), coord_ctx.inner_scale_sharing);
                }
            }

            // Merge channel_sharing_levels for x/y channels from coordination context
            // This ensures FacetContext has the correct per-channel scale sharing modes
            // computed from channel configs, enabling consistent axis visibility decisions.
            for (channel, level) in &coord_ctx.channel_sharing_levels {
                scale_sharing.insert(channel.clone(), ScaleSharing::from_level(*level));
            }
        }

        // Check if parent FacetContext exists and indicates same-type nesting
        // This is used for both global edge tracking and unified_channels modification
        let parent_facet_ctx = FacetContext::from_params(&self.base_params);
        let is_same_type_nesting = {
            let default_unified: std::collections::HashSet<&str> =
                DimConfig::unified_channels().iter().copied().collect();
            parent_facet_ctx
                .as_ref()
                .map(|ctx| {
                    default_unified
                        .iter()
                        .any(|ch| ctx.unified_channels.contains(*ch))
                })
                .unwrap_or(false)
        };

        // Compute global_edge_tracked_channels and global_edge_channels for same-type nesting.
        // global_edge_tracked_channels: channels using global edge logic (not filtered by position)
        // global_edge_channels: subset that are actually at the global edge position
        let (global_edge_tracked_channels, global_edge_channels) = {
            // Get the default unified channels for this facet type
            let default_unified: std::collections::HashSet<&str> =
                DimConfig::unified_channels().iter().copied().collect();

            // Find additional unified channels (from unified_x_title or unified_y_title)
            // These are channels in unified_channels that are NOT in the default set
            let additional_unified: std::collections::HashSet<String> = unified_channels
                .iter()
                .filter(|ch| !default_unified.contains(ch.as_str()))
                .cloned()
                .collect();

            // Use the pre-computed same-type nesting detection
            let parent_ctx = parent_facet_ctx.as_ref();

            // For same-type nesting, also check if orthogonal channel has Level(2+) sharing.
            // Level(2+) means sharing across multiple nesting levels, so we need global edge logic.
            // For Row faceting, orthogonal is "x"; for Col faceting, orthogonal is "y".
            let orthogonal_channel = if DimConfig::is_row_facet() { "x" } else { "y" };
            let orthogonal_has_multilevel_sharing = scale_sharing
                .get(orthogonal_channel)
                .map(|s| matches!(s, ScaleSharing::Level(n) if *n >= 2))
                .unwrap_or(false);

            // Channels that need global edge tracking: additional unified + orthogonal with Level(2+)
            let mut channels_needing_global_edge = additional_unified.clone();
            // Add orthogonal channel if it has multilevel sharing AND:
            // - We're in same-type nesting (parent has default channel unified), OR
            // - There's no parent (top level) - need to start tracking for child levels
            if orthogonal_has_multilevel_sharing && (is_same_type_nesting || parent_ctx.is_none()) {
                channels_needing_global_edge.insert(orthogonal_channel.to_string());
            }

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "SubplotIterator global_edge: channel={} is_same_type={} additional_unified={:?} orthogonal={} has_multilevel={} channels_needing={:?}",
                    DimConfig::channel_name(),
                    is_same_type_nesting,
                    additional_unified,
                    orthogonal_channel,
                    orthogonal_has_multilevel_sharing,
                    channels_needing_global_edge
                );
            }

            if is_same_type_nesting && !channels_needing_global_edge.is_empty() {
                // Same-type nesting with channels needing global edge logic
                // Inherit parent's tracked channels, adding any new ones we need to track
                let mut tracked = parent_ctx
                    .as_ref()
                    .map(|ctx| ctx.global_edge_tracked_channels.clone())
                    .unwrap_or_default();
                tracked.extend(channels_needing_global_edge.iter().cloned());

                // Check if parent was already tracking global edges
                let parent_was_tracking = parent_ctx
                    .as_ref()
                    .map(|ctx| !ctx.global_edge_tracked_channels.is_empty())
                    .unwrap_or(false);

                // Get parent's global_edge_channels - if parent wasn't tracking, start fresh
                // If parent WAS tracking but has empty global_edge_channels, child inherits empty
                let parent_edge = if parent_was_tracking {
                    parent_ctx
                        .as_ref()
                        .map(|ctx| ctx.global_edge_channels.clone())
                        .unwrap_or_default()
                } else {
                    // Parent wasn't tracking, so this is first level - start with all tracked
                    channels_needing_global_edge.clone()
                };

                let (row, col) = position;
                let (num_rows, _num_cols) = grid_dimensions;

                // Filter to only channels at local edge (and that we're tracking)
                let at_edge: std::collections::HashSet<String> = parent_edge
                    .into_iter()
                    .filter(|ch| tracked.contains(ch))
                    .filter(|ch| {
                        match ch.as_str() {
                            "x" => row == num_rows - 1, // Bottom edge
                            "y" => col == 0,           // Left edge
                            _ => true,
                        }
                    })
                    .collect();

                (tracked, at_edge)
            } else if parent_ctx.is_none() && !channels_needing_global_edge.is_empty() {
                // Top-level facet with channels needing global edge (no parent facet context)
                // Filter to only channels at edge based on position
                let (row, col) = position;
                let (num_rows, _num_cols) = grid_dimensions;
                let at_edge: std::collections::HashSet<String> = channels_needing_global_edge
                    .iter()
                    .filter(|ch| match ch.as_str() {
                        "x" => row == num_rows - 1, // Bottom edge
                        "y" => col == 0,            // Left edge
                        _ => true,
                    })
                    .cloned()
                    .collect();
                (channels_needing_global_edge, at_edge)
            } else {
                // No channels need global edge tracking
                // Don't apply global edge logic
                (std::collections::HashSet::new(), std::collections::HashSet::new())
            }
        };

        // For same-type nesting (Col>Col or Row>Row) with Level(2+) orthogonal channel sharing,
        // add that channel to unified_channels to suppress subplot axis titles.
        // The facet guide handles the unified title rendering at the global edge.
        // Only do this when:
        // 1. We're in same-type nesting (parent has the same default unified channels as us)
        // 2. Global edge tracking is active for the channel
        // For mixed nesting (Col>Row>Col), the Row facet handles unified y, so don't add here.
        if is_same_type_nesting {
            for ch in &global_edge_tracked_channels {
                unified_channels.insert(ch.clone());
            }
        }

        // Compute grammar-based visibility if partition_list is available
        let grammar_visibility = coordination_ctx
            .as_ref()
            .and_then(|ctx| ctx.partition_list.as_ref())
            .map(|partition_list| {
                // Build params with the current facet value added
                let mut params_with_facet = self.base_params.clone();
                params_with_facet.insert(DimConfig::channel_name().to_string(), facet_value.clone());

                // Build SubplotIndex from params
                let subplot_index = build_subplot_index_from_params(partition_list, &params_with_facet);

                // Get sharing levels from scale_sharing
                let x_sharing = scale_sharing
                    .get("x")
                    .map(|s| s.to_level())
                    .unwrap_or(0);
                let y_sharing = scale_sharing
                    .get("y")
                    .map(|s| s.to_level())
                    .unwrap_or(0);

                // Compute visibility (use Bottom for x, Left for y as defaults)
                compute_subplot_visibility(
                    partition_list,
                    &subplot_index,
                    x_sharing,
                    y_sharing,
                    AxisPosition::Bottom,
                    AxisPosition::Left,
                )
            });

        let facet_ctx = FacetContext {
            position,
            grid_dimensions,
            unified_channels,
            scale_sharing,
            global_edge_tracked_channels,
            global_edge_channels,
            grammar_visibility,
        };

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() && !facet_ctx.global_edge_channels.is_empty() {
            eprintln!(
                "SubplotIterator: created FacetContext with global_edge_channels={:?} position={:?}",
                facet_ctx.global_edge_channels, facet_ctx.position
            );
        }

        // Merge FacetContext into params
        let mut params = self.base_params.clone();
        params.extend(facet_ctx.to_params());

        self.current_index += 1;

        Some(SubplotIteration {
            index,
            facet_value,
            params,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.domain_vals.len() - self.current_index;
        (remaining, Some(remaining))
    }
}

impl<DimConfig: FacetDimensionConfig> ExactSizeIterator for SubplotIterator<DimConfig> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facet::dimension_config::RowDimensionConfig;
    use datafusion::common::ScalarValue;

    #[test]
    fn test_subplot_iterator_creates_correct_context() {
        let domain_vals = vec![
            ScalarValue::Utf8(Some("setosa".into())),
            ScalarValue::Utf8(Some("versicolor".into())),
            ScalarValue::Utf8(Some("virginica".into())),
        ];
        let params = IndexMap::new();
        let scale_sharing = std::collections::HashMap::new();

        let iter =
            SubplotIterator::<RowDimensionConfig>::new(domain_vals.clone(), params, scale_sharing, None);
        let items: Vec<_> = iter.collect();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].index, 0);
        assert_eq!(items[1].index, 1);
        assert_eq!(items[2].index, 2);

        // Verify facet values match
        assert_eq!(items[0].facet_value, domain_vals[0]);
        assert_eq!(items[1].facet_value, domain_vals[1]);
        assert_eq!(items[2].facet_value, domain_vals[2]);

        // Verify FacetContext in params
        use crate::facet::context::FacetContext;

        let ctx0 = FacetContext::from_params(&items[0].params).unwrap();
        assert_eq!(ctx0.position, (0, 0));
        assert_eq!(ctx0.grid_dimensions, (3, 1));
        assert!(ctx0.is_channel_unified("y"));
        assert!(!ctx0.is_channel_unified("x"));

        let ctx1 = FacetContext::from_params(&items[1].params).unwrap();
        assert_eq!(ctx1.position, (1, 0));
        assert_eq!(ctx1.grid_dimensions, (3, 1));
        assert!(ctx1.is_channel_unified("y"));
        assert!(!ctx1.is_channel_unified("x"));

        let ctx2 = FacetContext::from_params(&items[2].params).unwrap();
        assert_eq!(ctx2.position, (2, 0));
        assert_eq!(ctx2.grid_dimensions, (3, 1));
        assert!(ctx2.is_channel_unified("y"));
        assert!(!ctx2.is_channel_unified("x"));
    }

    #[test]
    fn test_subplot_iterator_unified_channels() {
        let domain_vals = vec![
            ScalarValue::Utf8(Some("A".into())),
            ScalarValue::Utf8(Some("B".into())),
        ];
        let params = IndexMap::new();
        let scale_sharing = std::collections::HashMap::new();

        let iter = SubplotIterator::<RowDimensionConfig>::new(domain_vals, params, scale_sharing, None);
        let items: Vec<_> = iter.collect();

        assert_eq!(items.len(), 2);

        use crate::facet::context::FacetContext;
        let ctx0 = FacetContext::from_params(&items[0].params).unwrap();
        // RowDimensionConfig unifies "y" channel
        assert!(ctx0.is_channel_unified("y"));
        assert!(!ctx0.is_channel_unified("x"));
    }

    #[test]
    fn test_subplot_iterator_size_hint() {
        let domain_vals = vec![
            ScalarValue::Utf8(Some("A".into())),
            ScalarValue::Utf8(Some("B".into())),
            ScalarValue::Utf8(Some("C".into())),
        ];
        let params = IndexMap::new();
        let scale_sharing = std::collections::HashMap::new();

        let mut iter =
            SubplotIterator::<RowDimensionConfig>::new(domain_vals, params, scale_sharing, None);
        assert_eq!(iter.size_hint(), (3, Some(3)));

        iter.next();
        assert_eq!(iter.size_hint(), (2, Some(2)));

        iter.next();
        assert_eq!(iter.size_hint(), (1, Some(1)));

        iter.next();
        assert_eq!(iter.size_hint(), (0, Some(0)));
    }

    #[test]
    fn test_subplot_iterator_merges_base_params() {
        let domain_vals = vec![ScalarValue::Utf8(Some("A".into()))];
        let mut base_params = IndexMap::new();
        base_params.insert("custom_param".to_string(), ScalarValue::Int32(Some(42)));
        let scale_sharing = std::collections::HashMap::new();

        let iter = SubplotIterator::<RowDimensionConfig>::new(
            domain_vals,
            base_params.clone(),
            scale_sharing,
            None,
        );
        let items: Vec<_> = iter.collect();

        assert_eq!(items.len(), 1);

        // Base params should be preserved
        assert_eq!(
            items[0].params.get("custom_param"),
            Some(&ScalarValue::Int32(Some(42)))
        );

        // FacetContext params should also be present
        use crate::facet::context::FacetContext;
        assert!(FacetContext::from_params(&items[0].params).is_some());
    }
}
