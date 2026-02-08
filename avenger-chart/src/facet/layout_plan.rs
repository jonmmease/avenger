//! Facet layout planning primitives and shared overflow routing helpers.
//!
//! This module is shared by facet coordinate measurement and facet guides to
//! keep cell planning and overflow edge/gap semantics consistent.

use crate::coords::OverflowSpaceRequirement;
use datafusion::{common::ScalarValue, logical_expr::Expr};

/// Canonical per-cell plan representation for column facet measurement.
#[derive(Clone, Debug)]
pub(crate) struct FacetCellPlan {
    pub value: ScalarValue,
    pub full_path: Vec<ScalarValue>,
    pub is_empty: bool,
    pub filter_predicate: Option<Expr>,
}

/// Resolved band-level facet layout values for a single FacetCol node.
#[derive(Clone, Debug, Default)]
pub(crate) struct FacetBandPlan {
    pub cells: Vec<FacetCellPlan>,
    pub padding_inner_px: f32,
    pub outer_left: f32,
    pub outer_right: f32,
    pub n: usize,
}

#[inline]
pub(crate) fn is_renderable_slot(is_empty: bool) -> bool {
    !is_empty
}

/// Compute `padding_inner_px` from the max of adjacent overflow combinations.
///
/// Pairs containing empty placeholder cells are skipped so hidden slots do not
/// inflate interior gaps.
pub(crate) fn compute_padding_from_overflows(
    overflows: &[OverflowSpaceRequirement],
    empty_cells: &[bool],
) -> f32 {
    if overflows.len() < 2 {
        return 0.0;
    }

    let mut max_padding = 0.0f32;
    for i in 0..overflows.len() - 1 {
        let left_is_empty = empty_cells.get(i).copied().unwrap_or(false);
        let right_is_empty = empty_cells.get(i + 1).copied().unwrap_or(false);
        if !is_renderable_slot(left_is_empty) || !is_renderable_slot(right_is_empty) {
            continue;
        }
        let combined = overflows[i].right + overflows[i + 1].left;
        max_padding = max_padding.max(combined);
    }
    max_padding
}

/// Compute effective first/last cell indices for outer-edge overflow routing.
///
/// Prefer first/last non-empty cells; fall back to raw edges if all are empty.
pub(crate) fn effective_edge_indices(empty_cells: &[bool], count: usize) -> Option<(usize, usize)> {
    if count == 0 {
        return None;
    }

    let first_non_empty =
        (0..count).find(|&idx| is_renderable_slot(empty_cells.get(idx).copied().unwrap_or(false)));
    let last_non_empty = (0..count)
        .rev()
        .find(|&idx| is_renderable_slot(empty_cells.get(idx).copied().unwrap_or(false)));

    match (first_non_empty, last_non_empty) {
        (Some(first), Some(last)) => Some((first, last)),
        _ => Some((0, count - 1)),
    }
}

pub(crate) fn effective_edge_indices_from_exists(
    exists_in_tree: &[bool],
) -> Option<(usize, usize)> {
    let empty_cells: Vec<bool> = exists_in_tree.iter().map(|exists| !*exists).collect();
    effective_edge_indices(&empty_cells, exists_in_tree.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_padding_skips_pairs_with_empty_cells() {
        let overflows = vec![
            OverflowSpaceRequirement {
                right: 0.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                right: 0.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 41.0,
                right: 0.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 0.0,
                ..Default::default()
            },
        ];
        let empty_cells = vec![true, true, false, false];
        let padding = compute_padding_from_overflows(&overflows, &empty_cells);
        assert_eq!(padding, 0.0);
    }

    #[test]
    fn compute_padding_uses_max_when_cells_non_empty() {
        let overflows = vec![
            OverflowSpaceRequirement {
                right: 7.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 5.0,
                right: 3.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 2.0,
                ..Default::default()
            },
        ];
        let empty_cells = vec![false, false, false];
        let padding = compute_padding_from_overflows(&overflows, &empty_cells);
        assert_eq!(padding, 12.0);
    }

    #[test]
    fn compute_padding_skips_middle_empty_pairs() {
        let overflows = vec![
            OverflowSpaceRequirement {
                right: 10.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 2.0,
                right: 4.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 6.0,
                right: 8.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 3.0,
                ..Default::default()
            },
        ];
        // pair 1-2 is skipped due to empty middle cell; max should come from pair 2-3
        let empty_cells = vec![false, true, false, false];
        let padding = compute_padding_from_overflows(&overflows, &empty_cells);
        assert_eq!(padding, 11.0);
    }

    #[test]
    fn compute_padding_ignores_trailing_empty_cell() {
        let overflows = vec![
            OverflowSpaceRequirement {
                right: 5.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 7.0,
                right: 9.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 13.0,
                ..Default::default()
            },
        ];
        // pair 1-2 should be ignored because right cell is empty
        let empty_cells = vec![false, false, true];
        let padding = compute_padding_from_overflows(&overflows, &empty_cells);
        assert_eq!(padding, 12.0);
    }

    #[test]
    fn compute_padding_returns_zero_when_all_slots_empty() {
        let overflows = vec![
            OverflowSpaceRequirement {
                right: 9.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 11.0,
                right: 7.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 6.0,
                ..Default::default()
            },
        ];
        let empty_cells = vec![true, true, true];
        let padding = compute_padding_from_overflows(&overflows, &empty_cells);
        assert_eq!(padding, 0.0);
    }

    #[test]
    fn effective_edge_indices_prefers_non_empty_cells() {
        let empty_cells = vec![true, true, false, false];
        assert_eq!(effective_edge_indices(&empty_cells, 4), Some((2, 3)));
    }

    #[test]
    fn effective_edge_indices_falls_back_when_all_empty() {
        let empty_cells = vec![true, true, true];
        assert_eq!(effective_edge_indices(&empty_cells, 3), Some((0, 2)));
    }

    #[test]
    fn effective_edge_indices_none_for_no_cells() {
        assert_eq!(effective_edge_indices(&[], 0), None);
    }

    #[test]
    fn effective_edge_indices_from_exists_maps_to_empty_mask() {
        let exists = vec![false, false, true, true];
        assert_eq!(effective_edge_indices_from_exists(&exists), Some((2, 3)));
    }
}
