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
use crate::facet::coordination::FacetCoordinationContext;
use crate::facet::dimension_config::FacetDimensionConfig;
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

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
    _phantom: std::marker::PhantomData<DimConfig>,
}

impl<DimConfig: FacetDimensionConfig> SubplotIterator<DimConfig> {
    /// Create a new subplot iterator
    ///
    /// # Arguments
    /// * `domain_vals` - The domain values to iterate over (one per subplot)
    /// * `base_params` - Base parameters to merge FacetContext into
    /// * `scale_sharing` - Per-channel scale sharing configuration (ScaleSharing enum)
    ///
    /// Note: unified_channels are automatically determined from DimConfig::unified_channels()
    pub fn new(
        domain_vals: Vec<ScalarValue>,
        base_params: IndexMap<String, ScalarValue>,
        scale_sharing: std::collections::HashMap<String, ScaleSharing>,
    ) -> Self {
        Self {
            domain_vals,
            base_params,
            scale_sharing,
            current_index: 0,
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
        let unified_channels = if let Some(parent_ctx) = FacetContext::from_params(&self.base_params) {
            // Merge parent's unified_channels with this dimension's
            let mut merged = parent_ctx.unified_channels;
            merged.extend(DimConfig::unified_channels());
            merged
        } else {
            // No parent context, use just this dimension's channels
            DimConfig::unified_channels()
        };

        // Check for FacetCoordinationContext from outer facet (for nested facets)
        // This provides outer_position and outer_count for computing proper grid positions
        let coordination_ctx = FacetCoordinationContext::from_params(&self.base_params);

        // Compute position and grid_dimensions
        // For nested facets, combine inner index with outer position
        let (position, grid_dimensions) = if let Some(ref coord_ctx) = coordination_ctx {
            // Check if this coordination context is meant for this channel
            let current_channel = DimConfig::channel_name();
            if coord_ctx.inner_channel.as_deref() == Some(current_channel) {
                // This coordination is for us - use outer position to compute true grid position
                // Use inner_domain_count from coordination context for consistent grid dimensions
                // across all outer subplots, even if some have fewer actual data values.
                let inner_count = if coord_ctx.inner_domain_count > 0 {
                    coord_ctx.inner_domain_count
                } else {
                    // Fallback to actual domain size if inner_domain_count not set
                    self.domain_vals.len()
                };

                let (pos, dims) = if DimConfig::is_row_facet() {
                    // This is FacetRow inside FacetColumn
                    // position: (row_index, column_position_from_outer)
                    // grid_dimensions: (num_rows, num_columns_from_outer)
                    let row = index;
                    let col = coord_ctx.outer_position;
                    let num_rows = inner_count;
                    let num_cols = coord_ctx.outer_count;
                    ((row, col), (num_rows, num_cols))
                } else {
                    // This is FacetColumn inside FacetRow
                    // position: (row_position_from_outer, column_index)
                    // grid_dimensions: (num_rows_from_outer, num_columns)
                    let row = coord_ctx.outer_position;
                    let col = index;
                    let num_rows = coord_ctx.outer_count;
                    let num_cols = inner_count;
                    ((row, col), (num_rows, num_cols))
                };
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "SubplotIterator: channel={} inner_channel={:?} coord matched! outer_pos={} outer_count={} inner_domain_count={} -> position=({},{}) grid=({},{})",
                        current_channel,
                        coord_ctx.inner_channel,
                        coord_ctx.outer_position,
                        coord_ctx.outer_count,
                        coord_ctx.inner_domain_count,
                        pos.0, pos.1,
                        dims.0, dims.1
                    );
                }
                (pos, dims)
            } else {
                // Coordination context is not for us (e.g., we're the outer facet) - use default
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "SubplotIterator: channel={} inner_channel={:?} - NOT FOR US, using default",
                        current_channel,
                        coord_ctx.inner_channel
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

        let facet_ctx = FacetContext {
            position,
            grid_dimensions,
            unified_channels,
            scale_sharing: self.scale_sharing.clone(),
        };

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
            SubplotIterator::<RowDimensionConfig>::new(domain_vals.clone(), params, scale_sharing);
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

        let iter = SubplotIterator::<RowDimensionConfig>::new(domain_vals, params, scale_sharing);
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
            SubplotIterator::<RowDimensionConfig>::new(domain_vals, params, scale_sharing);
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
