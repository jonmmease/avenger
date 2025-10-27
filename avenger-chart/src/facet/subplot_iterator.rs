//! Iterator abstraction for consistent subplot iteration in faceted layouts
//!
//! Ensures that every subplot iteration gets correct FacetContext with invariants enforced:
//! - position matches the iteration index
//! - grid_dimensions matches the total number of subplots
//! - unified_channel is computed consistently
//! - FacetContext is always present in params

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

/// Iterator over subplots in a faceted row layout
///
/// Ensures each subplot gets correct FacetContext with:
/// - position matching the iteration index
/// - grid_dimensions matching the total number of subplots
/// - unified_channel for proper axis handling
///
/// # Example
///
/// ```ignore
/// let iterator = SubplotIterator::new(
///     domain_vals,
///     unified_channel,
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
pub struct SubplotIterator {
    domain_vals: Vec<ScalarValue>,
    unified_channel: Option<String>,
    base_params: IndexMap<String, ScalarValue>,
    scale_sharing: std::collections::HashMap<String, bool>,
    current_index: usize,
}

impl SubplotIterator {
    /// Create a new subplot iterator
    ///
    /// # Arguments
    /// * `domain_vals` - The domain values to iterate over (one per subplot)
    /// * `unified_channel` - Which channel (if any) has a unified axis label
    /// * `base_params` - Base parameters to merge FacetContext into
    /// * `scale_sharing` - Per-channel scale sharing status (true = shared, false = independent)
    pub fn new(
        domain_vals: Vec<ScalarValue>,
        unified_channel: Option<String>,
        base_params: IndexMap<String, ScalarValue>,
        scale_sharing: std::collections::HashMap<String, bool>,
    ) -> Self {
        Self {
            domain_vals,
            unified_channel,
            base_params,
            scale_sharing,
            current_index: 0,
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

impl Iterator for SubplotIterator {
    type Item = SubplotIteration;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_index >= self.domain_vals.len() {
            return None;
        }

        let index = self.current_index;
        let facet_value = self.domain_vals[index].clone();

        // Create FacetContext with invariants enforced
        use crate::facet::context::FacetContext;
        let facet_ctx = FacetContext {
            position: (index, 0),
            grid_dimensions: (self.domain_vals.len(), 1),
            unified_channel: self.unified_channel.clone(),
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

impl ExactSizeIterator for SubplotIterator {}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::common::ScalarValue;

    #[test]
    fn test_subplot_iterator_creates_correct_context() {
        let domain_vals = vec![
            ScalarValue::Utf8(Some("setosa".into())),
            ScalarValue::Utf8(Some("versicolor".into())),
            ScalarValue::Utf8(Some("virginica".into())),
        ];
        let unified_channel = Some("y".to_string());
        let params = IndexMap::new();
        let scale_sharing = std::collections::HashMap::new();

        let iter = SubplotIterator::new(domain_vals.clone(), unified_channel.clone(), params, scale_sharing);
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
        assert_eq!(ctx0.unified_channel, Some("y".to_string()));

        let ctx1 = FacetContext::from_params(&items[1].params).unwrap();
        assert_eq!(ctx1.position, (1, 0));
        assert_eq!(ctx1.grid_dimensions, (3, 1));
        assert_eq!(ctx1.unified_channel, Some("y".to_string()));

        let ctx2 = FacetContext::from_params(&items[2].params).unwrap();
        assert_eq!(ctx2.position, (2, 0));
        assert_eq!(ctx2.grid_dimensions, (3, 1));
        assert_eq!(ctx2.unified_channel, Some("y".to_string()));
    }

    #[test]
    fn test_subplot_iterator_without_unified_channel() {
        let domain_vals = vec![
            ScalarValue::Utf8(Some("A".into())),
            ScalarValue::Utf8(Some("B".into())),
        ];
        let params = IndexMap::new();
        let scale_sharing = std::collections::HashMap::new();

        let iter = SubplotIterator::new(domain_vals, None, params, scale_sharing);
        let items: Vec<_> = iter.collect();

        assert_eq!(items.len(), 2);

        use crate::facet::context::FacetContext;
        let ctx0 = FacetContext::from_params(&items[0].params).unwrap();
        assert_eq!(ctx0.unified_channel, None);
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

        let mut iter = SubplotIterator::new(domain_vals, None, params, scale_sharing);
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

        let iter = SubplotIterator::new(domain_vals, None, base_params.clone(), scale_sharing);
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
