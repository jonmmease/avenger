//! Helper functions for phantom cell positioning in uniform free scaling
//!
//! When uniform free scaling is enabled, facets with fewer cells than the maximum
//! need to insert phantom cells to maintain consistent band sizing. This module
//! provides utilities for computing phantom placement.
//!
//! The placement strategy depends on `band_align`:
//! - When `band_align >= 0.5`: Phantoms are prepended (actual data at bottom/end)
//! - When `band_align < 0.5`: Phantoms are appended (actual data at top/start)

use datafusion::common::ScalarValue;

/// Information about phantom cell placement for uniform free scaling
#[derive(Debug, Clone, PartialEq)]
pub struct PhantomPlacement {
    /// Number of phantom cells to add
    pub phantom_count: usize,
    /// Whether phantoms should be prepended (true) or appended (false)
    pub prepend: bool,
}

impl PhantomPlacement {
    /// Compute phantom placement given band alignment and cell counts
    ///
    /// # Arguments
    /// * `band_align` - Band alignment (0.0 to 1.0, typically 0.0 or 1.0)
    /// * `actual_count` - Number of actual data cells
    /// * `uniform_count` - Target uniform count (max cells across all facets)
    ///
    /// # Returns
    /// `PhantomPlacement` with count and positioning information
    pub fn compute(band_align: f32, actual_count: usize, uniform_count: usize) -> Self {
        let phantom_count = uniform_count.saturating_sub(actual_count);
        let prepend = band_align >= 0.5;
        Self {
            phantom_count,
            prepend,
        }
    }

    /// Compute number of phantoms that would be prepended
    ///
    /// This is useful for adjusting row/column indices when phantoms are at the start.
    pub fn prepend_count(&self) -> usize {
        if self.prepend { self.phantom_count } else { 0 }
    }

    /// Create a padded domain by inserting placeholder values
    ///
    /// # Arguments
    /// * `actual_domain` - The actual domain values
    /// * `template` - Template value for creating placeholders (uses same type)
    ///
    /// # Returns
    /// Padded domain with phantoms either prepended or appended
    pub fn pad_domain(
        &self,
        actual_domain: &[ScalarValue],
        template: &ScalarValue,
    ) -> Vec<ScalarValue> {
        if self.phantom_count == 0 {
            return actual_domain.to_vec();
        }

        let phantoms: Vec<ScalarValue> = (0..self.phantom_count)
            .map(|i| match template {
                ScalarValue::Utf8View(_) => {
                    ScalarValue::Utf8View(Some(format!("__placeholder_{}", i)))
                }
                ScalarValue::Utf8(_) => ScalarValue::Utf8(Some(format!("__placeholder_{}", i))),
                _ => ScalarValue::Utf8(Some(format!("__placeholder_{}", i))),
            })
            .collect();

        if self.prepend {
            // Prepend phantoms: [phantom0, phantom1, ..., actual0, actual1, ...]
            let mut result = phantoms;
            result.extend(actual_domain.iter().cloned());
            result
        } else {
            // Append phantoms: [actual0, actual1, ..., phantom0, phantom1, ...]
            let mut result = actual_domain.to_vec();
            result.extend(phantoms);
            result
        }
    }

    /// Extract actual items from a padded collection by removing phantom positions
    ///
    /// # Arguments
    /// * `padded` - Collection that includes phantom positions
    ///
    /// # Returns
    /// Iterator over only the actual (non-phantom) items
    pub fn extract_actual<T>(&self, padded: Vec<T>) -> Vec<T> {
        if self.phantom_count == 0 || padded.len() <= self.phantom_count {
            return padded;
        }

        let actual_count = padded.len() - self.phantom_count;
        if self.prepend {
            // Phantoms at start, actual at end - take last N
            padded.into_iter().skip(self.phantom_count).collect()
        } else {
            // Phantoms at end, actual at start - take first N
            padded.into_iter().take(actual_count).collect()
        }
    }
}

/// Centralized phantom cell layout computation for facet evaluation
///
/// This struct encapsulates all phantom cell positioning logic, eliminating
/// duplication between Pass 1 (measurement) and Pass 2 (rendering).
///
/// # Usage
///
/// ```ignore
/// // In Pass 1: compute and store layout
/// let layout = PhantomCellLayout::compute(band_align, domain.len(), uniform_count);
/// pass1_result.phantom_layout = layout;
///
/// // In Pass 2: reuse the same layout
/// let updated_params = pass1.phantom_layout.update_params_with_phantom_context(&params);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PhantomCellLayout {
    /// Band alignment (0.0-1.0)
    pub band_align: f32,
    /// Phantom placement info
    pub placement: PhantomPlacement,
    /// Actual data cell count (excluding phantoms)
    pub actual_cell_count: usize,
    /// Uniform cell count (including phantoms), or actual_cell_count if no uniform sizing
    pub uniform_cell_count: usize,
}

impl PhantomCellLayout {
    /// Compute phantom layout from band alignment and cell counts
    ///
    /// # Arguments
    /// * `band_align` - Band alignment (0.0 to 1.0)
    /// * `actual_count` - Number of actual data cells
    /// * `uniform_count` - Optional target uniform count (max cells across all facets)
    ///
    /// # Returns
    /// `PhantomCellLayout` with computed placement and counts
    pub fn compute(band_align: f32, actual_count: usize, uniform_count: Option<usize>) -> Self {
        let uniform_cell_count = uniform_count.unwrap_or(actual_count);
        let placement = PhantomPlacement::compute(band_align, actual_count, uniform_cell_count);

        Self {
            band_align,
            placement,
            actual_cell_count: actual_count,
            uniform_cell_count,
        }
    }

    /// Number of phantom cells that are prepended
    ///
    /// Returns 0 if phantoms are appended or if no uniform sizing is active.
    pub fn prepend_count(&self) -> usize {
        self.placement.prepend_count()
    }

    /// Whether phantoms are prepended (true) or appended (false)
    pub fn prepends_phantoms(&self) -> bool {
        self.placement.prepend
    }

    /// Total number of phantom cells
    pub fn phantom_count(&self) -> usize {
        self.placement.phantom_count
    }

    /// Update params with phantom_prepend_count in coordination context
    ///
    /// This is used to communicate phantom positioning to SubplotIterator,
    /// enabling correct FacetContext.position computation for axis label visibility.
    ///
    /// # Arguments
    /// * `params` - Base params that may contain a FacetCoordinationContext
    ///
    /// # Returns
    /// Updated params with phantom_prepend_count set, or original params if no update needed
    pub fn update_params_with_phantom_context(
        &self,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ) -> indexmap::IndexMap<String, datafusion::common::ScalarValue> {
        use crate::facet::coordination::FacetCoordinationContext;

        let prepend_count = self.prepend_count();
        if prepend_count > 0 {
            if let Some(mut coord_ctx) = FacetCoordinationContext::from_params(params) {
                coord_ctx.phantom_prepend_count = prepend_count;
                let mut updated_params = params.clone();
                updated_params.extend(coord_ctx.to_params());
                return updated_params;
            }
        }
        params.clone()
    }

    /// Filter rects to exclude phantom positions
    ///
    /// When uniform free scaling creates extra band positions for phantoms,
    /// this method extracts only the actual data rects.
    ///
    /// # Arguments
    /// * `all_rects` - All rects including phantom positions
    ///
    /// # Returns
    /// Vector of rects corresponding only to actual data cells
    pub fn filter_actual_rects<T: Clone>(&self, all_rects: Vec<T>) -> Vec<T> {
        self.placement.extract_actual(all_rects)
    }
}

impl Default for PhantomCellLayout {
    fn default() -> Self {
        Self {
            band_align: 0.5,
            placement: PhantomPlacement {
                phantom_count: 0,
                prepend: false,
            },
            actual_cell_count: 0,
            uniform_cell_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_phantoms_needed() {
        let placement = PhantomPlacement::compute(0.5, 5, 5);
        assert_eq!(placement.phantom_count, 0);
        assert_eq!(placement.prepend_count(), 0);
    }

    #[test]
    fn test_prepend_phantoms_high_band_align() {
        let placement = PhantomPlacement::compute(1.0, 3, 5);
        assert_eq!(placement.phantom_count, 2);
        assert!(placement.prepend);
        assert_eq!(placement.prepend_count(), 2);
    }

    #[test]
    fn test_append_phantoms_low_band_align() {
        let placement = PhantomPlacement::compute(0.0, 3, 5);
        assert_eq!(placement.phantom_count, 2);
        assert!(!placement.prepend);
        assert_eq!(placement.prepend_count(), 0);
    }

    #[test]
    fn test_threshold_at_half() {
        // band_align = 0.5 should prepend
        let placement = PhantomPlacement::compute(0.5, 3, 5);
        assert!(placement.prepend);

        // band_align = 0.49 should append
        let placement = PhantomPlacement::compute(0.49, 3, 5);
        assert!(!placement.prepend);
    }

    #[test]
    fn test_pad_domain_prepend() {
        let placement = PhantomPlacement::compute(1.0, 2, 4);
        let domain = vec![
            ScalarValue::Utf8(Some("a".into())),
            ScalarValue::Utf8(Some("b".into())),
        ];
        let template = ScalarValue::Utf8(None);
        let padded = placement.pad_domain(&domain, &template);

        assert_eq!(padded.len(), 4);
        assert!(
            matches!(&padded[0], ScalarValue::Utf8(Some(s)) if s.starts_with("__placeholder_"))
        );
        assert!(
            matches!(&padded[1], ScalarValue::Utf8(Some(s)) if s.starts_with("__placeholder_"))
        );
        assert_eq!(padded[2], domain[0]);
        assert_eq!(padded[3], domain[1]);
    }

    #[test]
    fn test_pad_domain_append() {
        let placement = PhantomPlacement::compute(0.0, 2, 4);
        let domain = vec![
            ScalarValue::Utf8(Some("a".into())),
            ScalarValue::Utf8(Some("b".into())),
        ];
        let template = ScalarValue::Utf8(None);
        let padded = placement.pad_domain(&domain, &template);

        assert_eq!(padded.len(), 4);
        assert_eq!(padded[0], domain[0]);
        assert_eq!(padded[1], domain[1]);
        assert!(
            matches!(&padded[2], ScalarValue::Utf8(Some(s)) if s.starts_with("__placeholder_"))
        );
        assert!(
            matches!(&padded[3], ScalarValue::Utf8(Some(s)) if s.starts_with("__placeholder_"))
        );
    }

    #[test]
    fn test_extract_actual_prepend() {
        let placement = PhantomPlacement::compute(1.0, 2, 4);
        let padded = vec![1, 2, 3, 4]; // phantoms are 1,2 and actual are 3,4
        let actual = placement.extract_actual(padded);
        assert_eq!(actual, vec![3, 4]);
    }

    #[test]
    fn test_extract_actual_append() {
        let placement = PhantomPlacement::compute(0.0, 2, 4);
        let padded = vec![1, 2, 3, 4]; // actual are 1,2 and phantoms are 3,4
        let actual = placement.extract_actual(padded);
        assert_eq!(actual, vec![1, 2]);
    }

    #[test]
    fn test_extract_actual_no_phantoms() {
        let placement = PhantomPlacement::compute(0.5, 4, 4);
        let items = vec![1, 2, 3, 4];
        let actual = placement.extract_actual(items);
        assert_eq!(actual, vec![1, 2, 3, 4]);
    }

    // PhantomCellLayout tests

    #[test]
    fn test_layout_no_uniform_sizing() {
        let layout = PhantomCellLayout::compute(0.5, 5, None);
        assert_eq!(layout.actual_cell_count, 5);
        assert_eq!(layout.uniform_cell_count, 5);
        assert_eq!(layout.prepend_count(), 0);
        assert_eq!(layout.phantom_count(), 0);
    }

    #[test]
    fn test_layout_with_prepend() {
        let layout = PhantomCellLayout::compute(1.0, 3, Some(5));
        assert_eq!(layout.actual_cell_count, 3);
        assert_eq!(layout.uniform_cell_count, 5);
        assert_eq!(layout.prepend_count(), 2);
        assert_eq!(layout.phantom_count(), 2);
        assert!(layout.prepends_phantoms());
    }

    #[test]
    fn test_layout_with_append() {
        let layout = PhantomCellLayout::compute(0.0, 3, Some(5));
        assert_eq!(layout.actual_cell_count, 3);
        assert_eq!(layout.uniform_cell_count, 5);
        assert_eq!(layout.prepend_count(), 0);
        assert_eq!(layout.phantom_count(), 2);
        assert!(!layout.prepends_phantoms());
    }

    #[test]
    fn test_layout_filter_rects_prepend() {
        let layout = PhantomCellLayout::compute(1.0, 2, Some(4));
        let all_rects = vec!["p0", "p1", "a0", "a1"];
        let actual = layout.filter_actual_rects(all_rects);
        assert_eq!(actual, vec!["a0", "a1"]);
    }

    #[test]
    fn test_layout_filter_rects_append() {
        let layout = PhantomCellLayout::compute(0.0, 2, Some(4));
        let all_rects = vec!["a0", "a1", "p0", "p1"];
        let actual = layout.filter_actual_rects(all_rects);
        assert_eq!(actual, vec!["a0", "a1"]);
    }

    #[test]
    fn test_layout_default() {
        let layout = PhantomCellLayout::default();
        assert_eq!(layout.band_align, 0.5);
        assert_eq!(layout.actual_cell_count, 0);
        assert_eq!(layout.uniform_cell_count, 0);
        assert_eq!(layout.phantom_count(), 0);
    }
}
