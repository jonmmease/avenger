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
        if self.prepend {
            self.phantom_count
        } else {
            0
        }
    }

    /// Create a padded domain by inserting placeholder values
    ///
    /// # Arguments
    /// * `actual_domain` - The actual domain values
    /// * `template` - Template value for creating placeholders (uses same type)
    ///
    /// # Returns
    /// Padded domain with phantoms either prepended or appended
    pub fn pad_domain(&self, actual_domain: &[ScalarValue], template: &ScalarValue) -> Vec<ScalarValue> {
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
        assert!(matches!(&padded[0], ScalarValue::Utf8(Some(s)) if s.starts_with("__placeholder_")));
        assert!(matches!(&padded[1], ScalarValue::Utf8(Some(s)) if s.starts_with("__placeholder_")));
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
        assert!(matches!(&padded[2], ScalarValue::Utf8(Some(s)) if s.starts_with("__placeholder_")));
        assert!(matches!(&padded[3], ScalarValue::Utf8(Some(s)) if s.starts_with("__placeholder_")));
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
}
