//! Helper functions for facet guide measurement
//!
//! This module provides utilities for computing gaps between facet cells
//! and aggregating overflow measurements.

use crate::guide::OverflowSpaceRequirement;

/// Calculate the required gap between adjacent rows to prevent overflow overlap
///
/// For each pair of adjacent rows, the gap must be at least the sum of:
/// - bottom overflow of the upper row
/// - top overflow of the lower row
///
/// Returns the maximum required gap across all adjacent pairs, plus a safety margin.
///
/// # Arguments
/// * `overflows` - Overflow measurements for each row (top to bottom)
/// * `safety_margin` - Additional pixels to add to computed gap
///
/// # Returns
/// The required gap between rows in pixels
pub fn calculate_inter_row_gap(overflows: &[OverflowSpaceRequirement], safety_margin: f32) -> f32 {
    if overflows.len() < 2 {
        return 0.0;
    }

    let max_gap = overflows
        .windows(2)
        .map(|pair| pair[0].bottom + pair[1].top)
        .fold(0.0_f32, f32::max);

    max_gap + safety_margin
}

/// Calculate the required gap between adjacent columns to prevent overflow overlap
///
/// For each pair of adjacent columns, the gap must be at least the sum of:
/// - right overflow of the left column
/// - left overflow of the right column
///
/// Returns the maximum required gap across all adjacent pairs, plus a safety margin.
///
/// # Arguments
/// * `overflows` - Overflow measurements for each column (left to right)
/// * `safety_margin` - Additional pixels to add to computed gap
///
/// # Returns
/// The required gap between columns in pixels
pub fn calculate_inter_col_gap(overflows: &[OverflowSpaceRequirement], safety_margin: f32) -> f32 {
    if overflows.len() < 2 {
        return 0.0;
    }

    let max_gap = overflows
        .windows(2)
        .map(|pair| pair[0].right + pair[1].left)
        .fold(0.0_f32, f32::max);

    max_gap + safety_margin
}

/// Aggregate overflow measurements by taking the maximum of each component
///
/// # Arguments
/// * `overflows` - Overflow measurements to aggregate
///
/// # Returns
/// A single overflow with max of each component across all inputs
pub fn aggregate_overflow(overflows: &[OverflowSpaceRequirement]) -> OverflowSpaceRequirement {
    overflows
        .iter()
        .fold(OverflowSpaceRequirement::default(), |acc, o| {
            acc.max_components(o)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overflow(top: f32, bottom: f32, left: f32, right: f32) -> OverflowSpaceRequirement {
        OverflowSpaceRequirement {
            top,
            bottom,
            left,
            right,
        }
    }

    #[test]
    fn test_inter_row_gap_empty() {
        assert_eq!(calculate_inter_row_gap(&[], 0.0), 0.0);
    }

    #[test]
    fn test_inter_row_gap_single() {
        let overflows = vec![overflow(10.0, 20.0, 5.0, 5.0)];
        assert_eq!(calculate_inter_row_gap(&overflows, 0.0), 0.0);
    }

    #[test]
    fn test_inter_row_gap_two_rows() {
        let overflows = vec![
            overflow(10.0, 20.0, 5.0, 5.0), // row 0: bottom = 20
            overflow(15.0, 25.0, 5.0, 5.0), // row 1: top = 15
        ];
        // gap = row0.bottom + row1.top = 20 + 15 = 35
        assert_eq!(calculate_inter_row_gap(&overflows, 0.0), 35.0);
    }

    #[test]
    fn test_inter_row_gap_with_safety_margin() {
        let overflows = vec![
            overflow(10.0, 20.0, 5.0, 5.0),
            overflow(15.0, 25.0, 5.0, 5.0),
        ];
        assert_eq!(calculate_inter_row_gap(&overflows, 1.0), 36.0);
    }

    #[test]
    fn test_inter_row_gap_multiple_rows() {
        let overflows = vec![
            overflow(5.0, 10.0, 0.0, 0.0),  // row 0
            overflow(5.0, 30.0, 0.0, 0.0),  // row 1: gap 0->1 = 10+5 = 15
            overflow(20.0, 5.0, 0.0, 0.0),  // row 2: gap 1->2 = 30+20 = 50 (max)
        ];
        assert_eq!(calculate_inter_row_gap(&overflows, 0.0), 50.0);
    }

    #[test]
    fn test_inter_col_gap_two_columns() {
        let overflows = vec![
            overflow(5.0, 5.0, 10.0, 25.0), // col 0: right = 25
            overflow(5.0, 5.0, 15.0, 20.0), // col 1: left = 15
        ];
        // gap = col0.right + col1.left = 25 + 15 = 40
        assert_eq!(calculate_inter_col_gap(&overflows, 0.0), 40.0);
    }

    #[test]
    fn test_aggregate_overflow() {
        let overflows = vec![
            overflow(10.0, 20.0, 5.0, 15.0),
            overflow(5.0, 25.0, 10.0, 10.0),
            overflow(15.0, 10.0, 8.0, 20.0),
        ];
        let result = aggregate_overflow(&overflows);
        assert_eq!(result.top, 15.0);
        assert_eq!(result.bottom, 25.0);
        assert_eq!(result.left, 10.0);
        assert_eq!(result.right, 20.0);
    }

    #[test]
    fn test_aggregate_overflow_empty() {
        let result = aggregate_overflow(&[]);
        assert_eq!(result, OverflowSpaceRequirement::default());
    }
}
