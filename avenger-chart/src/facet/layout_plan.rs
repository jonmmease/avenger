//! Facet layout planning primitives and shared overflow routing helpers.
//!
//! This module is shared by facet coordinate measurement and facet guides to
//! keep cell planning and overflow edge/gap semantics consistent.

use crate::coords::{FacetAxis, OverflowSpaceRequirement};
use crate::facet::evaluated_facet_tree::EvaluatedFacetTree;
use datafusion::{common::ScalarValue, logical_expr::Expr};
use tracing::trace;

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
    pub padding_inner_px: f32,
    pub outer_start: f32,
    pub outer_end: f32,
    pub n: usize,
}

#[inline]
pub(crate) fn is_renderable_slot(is_empty: bool) -> bool {
    !is_empty
}

/// Compute `padding_inner_px` from interior edge demand maxima.
///
/// This intentionally excludes outermost guide/legend demand from interior
/// spacing by taking:
/// - the max incoming demand among non-first renderable cells
/// - the max outgoing demand among non-last renderable cells
///
/// Empty placeholder cells are ignored.
pub(crate) fn compute_padding_from_overflows(
    axis: FacetAxis,
    overflows: &[OverflowSpaceRequirement],
    empty_cells: &[bool],
) -> f32 {
    let Some((first_renderable_idx, last_renderable_idx)) =
        effective_edge_indices(empty_cells, overflows.len())
    else {
        return 0.0;
    };

    if first_renderable_idx >= last_renderable_idx {
        return 0.0;
    }

    let mut incoming_max = 0.0f32;
    let mut outgoing_max = 0.0f32;

    for i in first_renderable_idx..=last_renderable_idx {
        let is_empty = empty_cells.get(i).copied().unwrap_or(false);
        if !is_renderable_slot(is_empty) {
            continue;
        }

        let Some(overflow) = overflows.get(i) else {
            continue;
        };

        if i > first_renderable_idx {
            let incoming = match axis {
                FacetAxis::Column => overflow.left,
                FacetAxis::Row => overflow.top,
            };
            incoming_max = incoming_max.max(incoming);
        }

        if i < last_renderable_idx {
            let outgoing = match axis {
                FacetAxis::Column => overflow.right,
                FacetAxis::Row => overflow.bottom,
            };
            outgoing_max = outgoing_max.max(outgoing);
        }
    }

    trace!(
        ?axis,
        first_renderable_idx,
        last_renderable_idx,
        incoming_max,
        outgoing_max,
        padding_inner_px = incoming_max + outgoing_max,
        "Facet inner padding from interior edge demands"
    );

    incoming_max + outgoing_max
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

fn empty_mask_for_values_at_path(
    facet_tree: &EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    values: &[ScalarValue],
) -> Vec<bool> {
    values
        .iter()
        .map(|value| {
            let mut path = facet_path.to_vec();
            path.push(value.clone());
            !facet_tree.cell_exists(&path)
        })
        .collect()
}

pub(crate) fn effective_edge_indices_for_values_at_path(
    facet_tree: &EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    values: &[ScalarValue],
) -> Option<(usize, usize)> {
    let empty_cells = empty_mask_for_values_at_path(facet_tree, facet_path, values);
    effective_edge_indices(&empty_cells, values.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facet::evaluated_facet_tree::PartitionNode;
    use crate::guide::FacetDirection;

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
        let padding = compute_padding_from_overflows(FacetAxis::Column, &overflows, &empty_cells);
        assert_eq!(padding, 0.0);
    }

    #[test]
    fn column_inner_padding_uses_non_first_left_and_non_last_right_max() {
        let overflows = vec![
            OverflowSpaceRequirement {
                right: 30.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 1.0,
                right: 2.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 3.0,
                right: 4.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 40.0,
                ..Default::default()
            },
        ];
        let empty_cells = vec![false, false, false, false];
        let padding = compute_padding_from_overflows(FacetAxis::Column, &overflows, &empty_cells);
        assert_eq!(padding, 70.0);
    }

    #[test]
    fn column_inner_padding_ignores_first_left_and_last_right() {
        let overflows = vec![
            OverflowSpaceRequirement {
                left: 100.0,
                right: 5.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 7.0,
                right: 11.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 13.0,
                right: 200.0,
                ..Default::default()
            },
        ];
        let empty_cells = vec![false, false, false];
        let padding = compute_padding_from_overflows(FacetAxis::Column, &overflows, &empty_cells);
        assert_eq!(padding, 24.0);
    }

    #[test]
    fn inner_padding_respects_empty_placeholders() {
        let overflows = vec![
            OverflowSpaceRequirement {
                right: 9.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 99.0,
                right: 99.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 6.0,
                right: 1.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                left: 2.0,
                ..Default::default()
            },
        ];
        let empty_cells = vec![false, true, false, false];
        let padding = compute_padding_from_overflows(FacetAxis::Column, &overflows, &empty_cells);
        assert_eq!(padding, 15.0);
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
        let padding = compute_padding_from_overflows(FacetAxis::Column, &overflows, &empty_cells);
        assert_eq!(padding, 0.0);
    }

    #[test]
    fn compute_padding_for_row_uses_vertical_edges() {
        let overflows = vec![
            OverflowSpaceRequirement {
                bottom: 25.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                top: 1.0,
                bottom: 2.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                top: 3.0,
                bottom: 4.0,
                ..Default::default()
            },
            OverflowSpaceRequirement {
                top: 35.0,
                ..Default::default()
            },
        ];
        let empty_cells = vec![false, false, false, false];
        let padding = compute_padding_from_overflows(FacetAxis::Row, &overflows, &empty_cells);
        assert_eq!(padding, 60.0);
    }

    #[test]
    fn inner_padding_zero_with_single_renderable_slot() {
        let overflows = vec![
            OverflowSpaceRequirement {
                left: 10.0,
                right: 7.0,
                top: 6.0,
                bottom: 4.0,
            },
            OverflowSpaceRequirement {
                left: 20.0,
                right: 9.0,
                top: 8.0,
                bottom: 5.0,
            },
        ];
        let empty_cells = vec![true, false];
        assert_eq!(
            compute_padding_from_overflows(FacetAxis::Column, &overflows, &empty_cells),
            0.0
        );
        assert_eq!(
            compute_padding_from_overflows(FacetAxis::Row, &overflows, &empty_cells),
            0.0
        );
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

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    #[test]
    fn effective_edge_indices_for_values_at_path_uses_tree_existence() {
        let tree = EvaluatedFacetTree::new(Some(PartitionNode::leaf(
            FacetDirection::Column,
            255,
            "column".to_string(),
            None,
            vec![s("A"), s("B")],
        )));
        let values = vec![s("X"), s("A"), s("B")];
        let empty_mask = empty_mask_for_values_at_path(&tree, &[], &values);
        assert_eq!(empty_mask, vec![true, false, false]);
        assert_eq!(
            effective_edge_indices_for_values_at_path(&tree, &[], &values),
            Some((1, 2))
        );
    }
}
