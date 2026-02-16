//! Unified sharing policy helpers for faceting.
//!
//! This module centralizes sharing-level behavior used by both:
//! - domain coordination grouping keys, and
//! - axis visibility decisions.
//! - legend owner-cell visibility decisions.

use datafusion::common::ScalarValue;

use crate::{
    cartesian::axis::AxisPosition,
    facet::sharing_kernel::{self, SharingGroupEdge},
    guide::FacetDirection,
    legend::LegendPosition,
};

/// Compute the canonical domain-group key for a cell path.
///
/// Delegates to shared path math so domain coordination and visibility grouping
/// remain consistent.
pub(crate) fn domain_group_key(
    full_cell_path: &[ScalarValue],
    sharing_level: u8,
    facet_depth: u8,
) -> Vec<ScalarValue> {
    sharing_kernel::domain_group_key(full_cell_path, sharing_level, facet_depth)
}

/// Determine whether axis labels should be visible for a facet cell.
pub(crate) fn show_axis_labels(
    position_indices: &[usize],
    level_counts: &[usize],
    facet_depth: u8,
    sharing_level: u8,
    direction: FacetDirection,
    axis_position: AxisPosition,
) -> bool {
    if let Some(edge) = axis_edge_for_position(direction, axis_position) {
        sharing_kernel::owner_for_edge_with_sharing(
            edge,
            position_indices,
            level_counts,
            facet_depth,
            sharing_level,
        )
    } else {
        true
    }
}

/// Determine whether axis titles should be visible for a facet cell.
///
/// Titles follow global edge visibility (Shared semantics).
pub(crate) fn show_axis_title(
    position_indices: &[usize],
    level_counts: &[usize],
    facet_depth: u8,
    direction: FacetDirection,
    axis_position: AxisPosition,
) -> bool {
    let shared_level = 255;
    if let Some(edge) = axis_edge_for_position(direction, axis_position) {
        sharing_kernel::owner_for_edge_with_sharing(
            edge,
            position_indices,
            level_counts,
            facet_depth,
            shared_level,
        )
    } else {
        true
    }
}

#[inline]
fn axis_edge_for_position(
    direction: FacetDirection,
    axis_position: AxisPosition,
) -> Option<SharingGroupEdge> {
    match (direction, axis_position) {
        (FacetDirection::Column, AxisPosition::Left) => Some(SharingGroupEdge::Start),
        (FacetDirection::Column, AxisPosition::Right) => Some(SharingGroupEdge::End),
        (FacetDirection::Row, AxisPosition::Top) => Some(SharingGroupEdge::Start),
        (FacetDirection::Row, AxisPosition::Bottom) => Some(SharingGroupEdge::End),
        _ => None,
    }
}

pub(crate) fn legend_edge_for_position(position: LegendPosition) -> SharingGroupEdge {
    match position {
        LegendPosition::Left | LegendPosition::Top => SharingGroupEdge::Start,
        LegendPosition::Right | LegendPosition::Bottom => SharingGroupEdge::End,
    }
}

pub(crate) fn legend_owner_for_position(
    position_indices: &[usize],
    level_counts: &[usize],
    facet_depth: u8,
    sharing_level: u8,
    legend_position: LegendPosition,
) -> bool {
    let edge = legend_edge_for_position(legend_position);
    sharing_kernel::owner_for_edge_with_sharing(
        edge,
        position_indices,
        level_counts,
        facet_depth,
        sharing_level,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(v.to_string()))
    }

    #[test]
    fn domain_group_key_matches_ancestor_semantics() {
        let path = vec![s("A"), s("B"), s("C"), s("D")];
        assert_eq!(domain_group_key(&path, 0, 4), path);
        assert_eq!(domain_group_key(&path, 1, 4), vec![s("A"), s("B"), s("C")]);
        assert_eq!(domain_group_key(&path, 2, 4), vec![s("A"), s("B")]);
        assert!(domain_group_key(&path, 4, 4).is_empty());
    }

    #[test]
    fn labels_visibility_obeys_level_grouping_for_column_left() {
        let counts = vec![2, 4, 2, 2];
        let facet_depth = 4;

        // Level(1): groups are based on prefix of length 3, so suffix is just the
        // last index. [0,0,1,0] is first in its group while [0,0,1,1] is not.
        assert!(show_axis_labels(
            &[0, 0, 1, 0],
            &counts,
            facet_depth,
            1,
            FacetDirection::Column,
            AxisPosition::Left
        ));
        assert!(!show_axis_labels(
            &[0, 0, 1, 1],
            &counts,
            facet_depth,
            1,
            FacetDirection::Column,
            AxisPosition::Left
        ));
    }

    #[test]
    fn labels_visibility_handles_left_and_right_edges_under_shared() {
        let counts = vec![3];
        let facet_depth = 1;

        assert!(show_axis_labels(
            &[0],
            &counts,
            facet_depth,
            255,
            FacetDirection::Column,
            AxisPosition::Left
        ));
        assert!(!show_axis_labels(
            &[1],
            &counts,
            facet_depth,
            255,
            FacetDirection::Column,
            AxisPosition::Left
        ));

        assert!(!show_axis_labels(
            &[1],
            &counts,
            facet_depth,
            255,
            FacetDirection::Column,
            AxisPosition::Right
        ));
        assert!(show_axis_labels(
            &[2],
            &counts,
            facet_depth,
            255,
            FacetDirection::Column,
            AxisPosition::Right
        ));
    }

    #[test]
    fn title_visibility_uses_global_edge_rules() {
        let counts = vec![2, 2];
        let facet_depth = 2;

        assert!(show_axis_title(
            &[0, 0],
            &counts,
            facet_depth,
            FacetDirection::Column,
            AxisPosition::Left
        ));
        assert!(!show_axis_title(
            &[0, 1],
            &counts,
            facet_depth,
            FacetDirection::Column,
            AxisPosition::Left
        ));
        assert!(show_axis_title(
            &[1, 1],
            &counts,
            facet_depth,
            FacetDirection::Column,
            AxisPosition::Right
        ));
    }

    #[test]
    fn domain_group_key_supports_null_values() {
        let path = vec![ScalarValue::Null, s("B"), ScalarValue::Int64(Some(5))];
        let key = domain_group_key(&path, 1, 3);
        assert_eq!(key, vec![ScalarValue::Null, s("B")]);
    }

    #[test]
    fn legend_owner_matches_axis_start_end_semantics() {
        let counts = vec![2, 2];
        let facet_depth = 2;
        let idx_start = vec![1, 0];
        let idx_end = vec![1, 1];

        assert!(legend_owner_for_position(
            &idx_start,
            &counts,
            facet_depth,
            1,
            LegendPosition::Top
        ));
        assert!(legend_owner_for_position(
            &idx_end,
            &counts,
            facet_depth,
            1,
            LegendPosition::Bottom
        ));
        assert!(legend_owner_for_position(
            &idx_start,
            &counts,
            facet_depth,
            1,
            LegendPosition::Left
        ));
        assert!(legend_owner_for_position(
            &idx_end,
            &counts,
            facet_depth,
            1,
            LegendPosition::Right
        ));
    }
}
