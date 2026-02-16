//! Shared ownership and grouping kernel for facet sharing behavior.
//!
//! This module provides the canonical group-boundary and owner checks used by
//! axis visibility, legend visibility, and domain-group key computation.

use datafusion::common::ScalarValue;

use crate::facet::path_math;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SharingGroupEdge {
    Start,
    End,
}

#[inline]
pub(crate) fn group_boundary(facet_depth: u8, sharing_level: u8) -> usize {
    path_math::sharing_group_boundary(facet_depth, sharing_level)
}

#[inline]
pub(crate) fn is_group_start(position_indices: &[usize], boundary: usize) -> bool {
    position_indices
        .get(boundary..)
        .map(|suffix| suffix.iter().all(|&i| i == 0))
        .unwrap_or(true)
}

#[inline]
pub(crate) fn is_group_end(
    position_indices: &[usize],
    level_counts: &[usize],
    boundary: usize,
) -> bool {
    let Some(suffix) = position_indices.get(boundary..) else {
        return true;
    };

    let Some(count_suffix) = level_counts.get(boundary..boundary + suffix.len()) else {
        return false;
    };

    suffix
        .iter()
        .zip(count_suffix.iter())
        .all(|(&pos, &count)| pos == count.saturating_sub(1))
}

#[inline]
pub(crate) fn owner_for_edge(
    edge: SharingGroupEdge,
    position_indices: &[usize],
    level_counts: &[usize],
    boundary: usize,
) -> bool {
    match edge {
        SharingGroupEdge::Start => is_group_start(position_indices, boundary),
        SharingGroupEdge::End => is_group_end(position_indices, level_counts, boundary),
    }
}

#[inline]
pub(crate) fn owner_for_edge_with_sharing(
    edge: SharingGroupEdge,
    position_indices: &[usize],
    level_counts: &[usize],
    facet_depth: u8,
    sharing_level: u8,
) -> bool {
    let boundary = group_boundary(facet_depth, sharing_level);
    owner_for_edge(edge, position_indices, level_counts, boundary)
}

#[inline]
pub(crate) fn domain_group_key(
    full_cell_path: &[ScalarValue],
    sharing_level: u8,
    facet_depth: u8,
) -> Vec<ScalarValue> {
    path_math::ancestor_key(full_cell_path, sharing_level, facet_depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_for_start_checks_suffix_zero() {
        let indices = vec![1, 0, 0];
        assert!(owner_for_edge(
            SharingGroupEdge::Start,
            &indices,
            &[2, 2, 2],
            1
        ));
        assert!(!owner_for_edge(
            SharingGroupEdge::Start,
            &[1, 0, 1],
            &[2, 2, 2],
            1
        ));
    }

    #[test]
    fn owner_for_end_checks_suffix_last() {
        let indices = vec![1, 1, 2];
        let counts = vec![2, 2, 3];
        assert!(owner_for_edge(SharingGroupEdge::End, &indices, &counts, 1));
        assert!(!owner_for_edge(
            SharingGroupEdge::End,
            &[1, 0, 2],
            &counts,
            1
        ));
    }
}
