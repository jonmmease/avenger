//! Z-index layer partitioning algorithm
//!
//! This module implements an algorithm to partition z-indices into minimal non-overlapping
//! layers that respect both z-index ordering and document order.
//!
//! ## Problem
//!
//! When rendering marks with z-indices in document order, we need to ensure that marks
//! with lower z-indices are rendered before marks with higher z-indices. However, if the
//! z-indices are not in sorted order in the document, we need multiple rendering passes.
//!
//! ## Solution
//!
//! The algorithm partitions z-indices into non-overlapping intervals (layers) such that:
//! 1. Each z-index belongs to exactly one layer
//! 2. Within each layer, marks appear in document order
//! 3. Layers are non-overlapping and sorted by their z-index ranges
//! 4. The number of layers is minimized
//!
//! ## Example
//!
//! Input: `[0, 2, -1, 3, 1, 4]` (z-indices in document order)
//! Output:
//! - Layer 1: `(-1, -1)` - contains mark at position 2
//! - Layer 2: `(0, 1)` - contains marks at positions 0 and 4
//! - Layer 3: `(2, 4)` - contains marks at positions 1, 3, and 5

/// Compute minimal z-index layers from a sequence of z-indices
///
/// Given z-indices in document order, returns a vector of (min, max) tuples
/// representing non-overlapping layers that preserve both z-index and document order.
///
/// The algorithm partitions z-indices into minimal non-overlapping intervals where
/// values within each interval appear in ascending order in the document.
///
/// # Algorithm
///
/// 1. Create indices that would sort the input by z-value (argsort)
/// 2. Iterate through these indices in sorted z-value order
/// 3. Add each index to the current partition if it's greater than all previous indices
/// 4. Otherwise, start a new partition
///
/// # Complexity
///
/// O(n log n) where n is the number of z-indices, dominated by the sorting step.
pub fn compute_zindex_layers(z_indices: Vec<i32>) -> Vec<(i32, i32)> {
    if z_indices.is_empty() {
        return vec![];
    }

    // Create indices that would sort the z_indices vector
    let mut indices: Vec<usize> = (0..z_indices.len()).collect();
    indices.sort_by_key(|&i| z_indices[i]);

    // Build partitions by iterating through sorted indices
    let mut partitions = Vec::new();
    let mut current_partition = vec![indices[0]];
    let mut max_index_in_partition = indices[0];

    for &idx in &indices[1..] {
        if idx > max_index_in_partition {
            // Can add to current partition
            current_partition.push(idx);
            max_index_in_partition = idx;
        } else {
            // Need to start a new partition
            partitions.push(current_partition);
            current_partition = vec![idx];
            max_index_in_partition = idx;
        }
    }

    // Don't forget the last partition
    partitions.push(current_partition);

    // Convert partitions of indices to intervals of z-values
    partitions
        .into_iter()
        .map(|partition| {
            let z_values: Vec<i32> = partition.iter().map(|&i| z_indices[i]).collect();
            let min_z = *z_values.iter().min().unwrap();
            let max_z = *z_values.iter().max().unwrap();
            (min_z, max_z)
        })
        .collect()
}

/// Verify that a partition set is valid according to the problem constraints
///
/// Returns true if:
/// 1. The partitions are non-overlapping and in ascending order
/// 2. The values in each partition are sorted within the input vector
/// 3. Every value from the original vector is represented in exactly one partition
pub fn verify_partitions(z_indices: &[i32], partitions: &[(i32, i32)]) -> bool {
    // Check 1: Non-overlapping and ascending order
    for i in 1..partitions.len() {
        if partitions[i - 1].1 >= partitions[i].0 {
            return false; // Overlapping or not in ascending order
        }
    }

    // Check 2: Values in each partition are sorted within the input vector
    for &(min_z, max_z) in partitions {
        let mut last_pos = None;
        for (pos, &z) in z_indices.iter().enumerate() {
            if z >= min_z && z <= max_z {
                if let Some(prev_pos) = last_pos {
                    // Check that this z-value is >= the previous one in the partition
                    if z < z_indices[prev_pos] {
                        return false; // Not sorted within the partition
                    }
                }
                last_pos = Some(pos);
            }
        }
    }

    // Check 3: Every value is represented exactly once
    let mut found = vec![false; z_indices.len()];
    for &(min_z, max_z) in partitions {
        for (pos, &z) in z_indices.iter().enumerate() {
            if z >= min_z && z <= max_z {
                if found[pos] {
                    return false; // Value found in multiple partitions
                }
                found[pos] = true;
            }
        }
    }

    // All values must be found
    found.iter().all(|&f| f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        let result = compute_zindex_layers(vec![]);
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_single_element() {
        let result = compute_zindex_layers(vec![5]);
        assert_eq!(result, vec![(5, 5)]);
    }

    #[test]
    fn test_ascending_order() {
        // No splits needed when already in ascending order
        let result = compute_zindex_layers(vec![1, 2, 3, 4, 5]);
        assert_eq!(result, vec![(1, 5)]);
    }

    #[test]
    fn test_descending_order() {
        // Worst case - each element needs its own layer
        let result = compute_zindex_layers(vec![5, 4, 3, 2, 1]);
        assert_eq!(result, vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]);
    }

    #[test]
    fn test_example_from_spec() {
        // Example from the specification
        let z_indices = vec![0, 2, -1, 3, 1, 4];

        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(-1, -1), (0, 1), (2, 4)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_duplicates() {
        // Duplicate z-indices should be handled correctly
        let z_indices = vec![1, 2, 2, 3, 1, 4];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(1, 1), (2, 4)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_negative_values() {
        // Test with negative z-indices
        let z_indices = vec![-2, -1, 0, 1, -3, 2];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(-3, -3), (-2, 2)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_alternating_pattern() {
        let z_indices = vec![1, 10, 2, 9, 3, 8];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(1, 8), (9, 9), (10, 10)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_multiple_inversions() {
        // Multiple inversions requiring splits
        let z_indices = vec![5, 3, 7, 1, 9, 2];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(1, 2), (3, 3), (5, 9)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_complex_case() {
        let z_indices = vec![0, 5, 3, 8, 2, 6, 1, 7, 4, 9];
        let result = compute_zindex_layers(z_indices.clone());

        // Verify that result is valid
        assert!(verify_partitions(&z_indices, &result));

        // Verify that layers are non-overlapping and sorted
        for i in 1..result.len() {
            assert!(
                result[i - 1].1 < result[i].0,
                "Layers should be non-overlapping: {:?} and {:?}",
                result[i - 1],
                result[i]
            );
        }
    }

    #[test]
    fn test_large_range() {
        let z_indices = vec![1000, 2000, 500, 3000];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(500, 500), (1000, 3000)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_all_same() {
        // All z-indices are the same
        let result = compute_zindex_layers(vec![5, 5, 5, 5]);
        assert_eq!(result, vec![(5, 5)]);
    }

    #[test]
    fn test_two_groups() {
        let z_indices = vec![1, 2, 3, 10, 11, 12];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(1, 12)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_zigzag_pattern() {
        let z_indices = vec![1, 5, 2, 4, 3];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(1, 3), (4, 4), (5, 5)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_verify_partitions_valid() {
        // Test the verification function with valid partitions

        // Single element
        assert!(verify_partitions(&[5], &[(5, 5)]));

        // Ascending order - single partition
        assert!(verify_partitions(&[1, 2, 3, 4, 5], &[(1, 5)]));

        // Example from spec
        assert!(verify_partitions(
            &[0, 2, -1, 3, 1, 4],
            &[(-1, -1), (0, 1), (2, 4)]
        ));

        // Alternating pattern
        assert!(verify_partitions(
            &[1, 10, 2, 9, 3, 8],
            &[(1, 8), (9, 9), (10, 10)]
        ));

        // Descending order
        assert!(verify_partitions(
            &[5, 4, 3, 2, 1],
            &[(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]
        ));
    }

    #[test]
    fn test_verify_partitions_invalid() {
        // Test the verification function with invalid partitions

        // Overlapping partitions
        assert!(!verify_partitions(&[1, 2, 3, 4], &[(1, 2), (2, 4)]));

        // Not in ascending order
        assert!(!verify_partitions(&[1, 2, 3, 4], &[(3, 4), (1, 2)]));

        // Values not sorted within partition
        assert!(!verify_partitions(&[3, 1, 2], &[(1, 3)])); // 3,1,2 are not sorted

        // Missing values
        assert!(!verify_partitions(&[1, 2, 3, 4], &[(1, 2)])); // 3,4 not covered

        // Value in multiple partitions (if ranges overlapped)
        assert!(!verify_partitions(&[1, 2, 3], &[(1, 2), (2, 3)])); // 2 in both
    }

    #[test]
    fn test_all_expected_results_are_valid() {
        // Verify all our expected test results are valid partitions

        // Empty
        assert!(verify_partitions(&[], &[]));

        // Single element
        assert!(verify_partitions(&[5], &[(5, 5)]));

        // Ascending order
        assert!(verify_partitions(&[1, 2, 3, 4, 5], &[(1, 5)]));

        // Descending order
        assert!(verify_partitions(
            &[5, 4, 3, 2, 1],
            &[(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]
        ));

        // Example from spec
        assert!(verify_partitions(
            &[0, 2, -1, 3, 1, 4],
            &[(-1, -1), (0, 1), (2, 4)]
        ));

        // Duplicates
        assert!(verify_partitions(&[1, 2, 2, 3, 1, 4], &[(1, 1), (2, 4)]));

        // Negative values
        assert!(verify_partitions(
            &[-2, -1, 0, 1, -3, 2],
            &[(-3, -3), (-2, -1), (0, 2)]
        ));

        // Alternating pattern
        assert!(verify_partitions(
            &[1, 10, 2, 9, 3, 8],
            &[(1, 8), (9, 9), (10, 10)]
        ));

        // Multiple inversions
        assert!(verify_partitions(
            &[5, 3, 7, 1, 9, 2],
            &[(1, 1), (2, 2), (3, 3), (5, 9)]
        ));

        // Large range
        assert!(verify_partitions(
            &[1000, 2000, 500, 3000],
            &[(500, 500), (1000, 3000)]
        ));

        // All same
        assert!(verify_partitions(&[5, 5, 5, 5], &[(5, 5)]));

        // Two groups
        assert!(verify_partitions(&[1, 2, 3, 10, 11, 12], &[(1, 12)]));

        // Zigzag pattern
        assert!(verify_partitions(
            &[1, 5, 2, 4, 3],
            &[(1, 3), (4, 4), (5, 5)]
        ));
    }
}
