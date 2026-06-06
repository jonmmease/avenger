//! Facet layout planning primitives and shared overflow routing helpers.
//!
//! This module is shared by facet coordinate measurement and facet guides to
//! keep cell planning and overflow edge/gap semantics consistent.

use crate::facet::evaluated_facet_tree::EvaluatedFacetTree;
pub(crate) use crate::partition::{
    PartitionCellEmptyKind as FacetCellEmptyKind, PartitionCellPlan as FacetCellPlan,
};
use avenger_chart_core::{
    AxisOwnershipMode, AxisPosition, AxisVisibility, FacetAxis, FacetGuideSharingView,
    OverflowSpaceRequirement, SharingLevel,
};
use datafusion::common::ScalarValue;
use std::collections::HashMap;
use tracing::trace;

/// Resolved band-level facet layout values for a single facet band node.
#[derive(Clone, Debug, Default)]
pub(crate) struct FacetBandPlan {
    pub padding_inner_px: f32,
    pub guide_padding_inner_px: f32,
    pub outer_start: f32,
    pub outer_end: f32,
    pub n: usize,
}

/// Realized inner-padding lower bounds from a previous refinement pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FacetBandPaddingFeedback {
    pub padding_inner_px: f32,
    pub guide_padding_inner_px: f32,
}

pub(crate) type FacetBandPaddingFeedbackMap = HashMap<Vec<usize>, FacetBandPaddingFeedback>;

#[inline]
pub(crate) fn is_renderable_slot(renderable: bool) -> bool {
    renderable
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
    renderable_cells: &[bool],
) -> f32 {
    let Some((first_renderable_idx, last_renderable_idx)) =
        effective_edge_indices(renderable_cells, overflows.len())
    else {
        return 0.0;
    };

    if first_renderable_idx >= last_renderable_idx {
        return 0.0;
    }

    let mut incoming_max = 0.0f32;
    let mut outgoing_max = 0.0f32;

    for i in first_renderable_idx..=last_renderable_idx {
        let renderable = renderable_cells.get(i).copied().unwrap_or(false);
        if !is_renderable_slot(renderable) {
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
/// Prefer first/last renderable cells; fall back to raw edges if none are renderable.
pub(crate) fn effective_edge_indices(
    renderable_cells: &[bool],
    count: usize,
) -> Option<(usize, usize)> {
    if count == 0 {
        return None;
    }

    let first_non_empty = (0..count)
        .find(|&idx| is_renderable_slot(renderable_cells.get(idx).copied().unwrap_or(false)));
    let last_non_empty = (0..count)
        .rev()
        .find(|&idx| is_renderable_slot(renderable_cells.get(idx).copied().unwrap_or(false)));

    match (first_non_empty, last_non_empty) {
        (Some(first), Some(last)) => Some((first, last)),
        _ => Some((0, count - 1)),
    }
}

fn renderable_mask_for_values_at_path(
    facet_tree: &EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    values: &[ScalarValue],
) -> Vec<bool> {
    values
        .iter()
        .map(|value| {
            let mut path = facet_path.to_vec();
            path.push(value.clone());
            facet_tree.cell_exists(&path)
        })
        .collect()
}

pub(crate) fn effective_edge_indices_for_values_at_path(
    facet_tree: &EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    values: &[ScalarValue],
) -> Option<(usize, usize)> {
    let renderable_cells = renderable_mask_for_values_at_path(facet_tree, facet_path, values);
    effective_edge_indices(&renderable_cells, values.len())
}

impl FacetGuideSharingView for EvaluatedFacetTree {
    fn channel_axis_visibility_for_path_checked(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
        sharing_level: u8,
    ) -> Option<AxisVisibility> {
        self.channel_axis_visibility_for_path_checked(path, axis_position, sharing_level)
    }

    fn channel_axis_visibility_for_path_checked_with_mode(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
        sharing_level: u8,
        ownership_mode: AxisOwnershipMode,
    ) -> Option<AxisVisibility> {
        self.channel_axis_visibility_for_path_checked_with_mode(
            path,
            axis_position,
            sharing_level,
            ownership_mode,
        )
    }

    fn is_jagged_for_axis(&self, axis_position: AxisPosition) -> bool {
        self.is_jagged_for_axis(axis_position)
    }

    fn channel_domain_sharing_level(&self, channel: &str) -> SharingLevel {
        self.channel_domain_sharing_level_typed(channel)
    }

    fn axis_guide_visibility_config_for_path(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
    ) -> Option<avenger_chart_core::AxisGuideVisibilityConfig> {
        self.axis_guide_visibility_config_for_path(path, axis_position)
    }

    fn effective_edge_indices_for_values_at_path(
        &self,
        facet_path: &[ScalarValue],
        values: &[ScalarValue],
    ) -> Option<(usize, usize)> {
        effective_edge_indices_for_values_at_path(self, facet_path, values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facet::FacetDirection;
    use crate::facet::evaluated_facet_tree::PartitionNode;

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
        let renderable_cells = vec![false, false, true, true];
        let padding =
            compute_padding_from_overflows(FacetAxis::Column, &overflows, &renderable_cells);
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
        let renderable_cells = vec![true, true, true, true];
        let padding =
            compute_padding_from_overflows(FacetAxis::Column, &overflows, &renderable_cells);
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
        let renderable_cells = vec![true, true, true];
        let padding =
            compute_padding_from_overflows(FacetAxis::Column, &overflows, &renderable_cells);
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
        let renderable_cells = vec![true, false, true, true];
        let padding =
            compute_padding_from_overflows(FacetAxis::Column, &overflows, &renderable_cells);
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
        let renderable_cells = vec![false, false, false];
        let padding =
            compute_padding_from_overflows(FacetAxis::Column, &overflows, &renderable_cells);
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
        let renderable_cells = vec![true, true, true, true];
        let padding = compute_padding_from_overflows(FacetAxis::Row, &overflows, &renderable_cells);
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
        let renderable_cells = vec![false, true];
        assert_eq!(
            compute_padding_from_overflows(FacetAxis::Column, &overflows, &renderable_cells),
            0.0
        );
        assert_eq!(
            compute_padding_from_overflows(FacetAxis::Row, &overflows, &renderable_cells),
            0.0
        );
    }

    #[test]
    fn effective_edge_indices_prefers_non_empty_cells() {
        let renderable_cells = vec![false, false, true, true];
        assert_eq!(effective_edge_indices(&renderable_cells, 4), Some((2, 3)));
    }

    #[test]
    fn effective_edge_indices_falls_back_when_all_empty() {
        let renderable_cells = vec![false, false, false];
        assert_eq!(effective_edge_indices(&renderable_cells, 3), Some((0, 2)));
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
        let renderable_mask = renderable_mask_for_values_at_path(&tree, &[], &values);
        assert_eq!(renderable_mask, vec![false, true, true]);
        assert_eq!(
            effective_edge_indices_for_values_at_path(&tree, &[], &values),
            Some((1, 2))
        );
    }
}
