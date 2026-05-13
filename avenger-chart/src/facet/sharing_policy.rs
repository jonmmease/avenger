//! Unified sharing policy helpers for faceting.
//!
//! This module centralizes sharing-level behavior used by:
//! - domain coordination grouping keys,
//! - axis visibility decisions.
//! - legend owner-cell visibility decisions.

use datafusion::common::ScalarValue;

use crate::{
    cartesian::axis::AxisPosition,
    facet::{
        sharing_kernel::{self, SharingGroupEdge},
        sharing_level::SharingLevel,
    },
    guide::FacetDirection,
    legend::LegendPosition,
};

/// Compute the canonical domain-group key for a cell path.
///
/// Delegates to shared path math so domain coordination and visibility grouping
/// remain consistent.
pub(crate) fn domain_group_key(
    full_cell_path: &[ScalarValue],
    sharing_level: SharingLevel,
    facet_depth: u8,
) -> Vec<ScalarValue> {
    sharing_kernel::domain_group_key(full_cell_path, sharing_level, facet_depth)
}

/// Determine whether axis labels should be visible for a facet cell.
pub(crate) fn show_axis_labels(
    position_indices: &[usize],
    level_counts: &[usize],
    facet_depth: u8,
    sharing_level: SharingLevel,
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
    let shared_level = SharingLevel::GLOBAL;
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

/// Determine whether cartesian axis labels should be visible for a facet cell.
///
/// Visibility is projected to the axis-relevant facet direction while preserving
/// orthogonal-strip grouping:
/// - x axes are controlled by row-facet levels
/// - y axes are controlled by column-facet levels
pub(crate) fn show_cartesian_axis_labels(
    position_indices: &[usize],
    level_counts: &[usize],
    level_directions: &[FacetDirection],
    axis_position: AxisPosition,
    sharing_level: SharingLevel,
) -> bool {
    let Some((projected_indices, projected_counts, relevant_depth, edge)) =
        project_levels_for_cartesian_axis(
            position_indices,
            level_counts,
            level_directions,
            axis_position,
        )
    else {
        return true;
    };

    if relevant_depth == 0 {
        return true;
    }

    let labels_sharing = sharing_level.clamp_to_depth(relevant_depth as u8);
    sharing_kernel::owner_for_edge_with_sharing(
        edge,
        &projected_indices,
        &projected_counts,
        projected_indices.len() as u8,
        labels_sharing,
    )
}

/// Determine whether cartesian axis titles should be visible for a facet cell.
///
/// Titles are shared over axis-relevant levels only, while remaining independent
/// across orthogonal strips.
pub(crate) fn show_cartesian_axis_title(
    position_indices: &[usize],
    level_counts: &[usize],
    level_directions: &[FacetDirection],
    axis_position: AxisPosition,
) -> bool {
    let Some((projected_indices, projected_counts, relevant_depth, edge)) =
        project_levels_for_cartesian_axis(
            position_indices,
            level_counts,
            level_directions,
            axis_position,
        )
    else {
        return true;
    };

    if relevant_depth == 0 {
        return true;
    }

    sharing_kernel::owner_for_edge_with_sharing(
        edge,
        &projected_indices,
        &projected_counts,
        projected_indices.len() as u8,
        SharingLevel::from_raw(relevant_depth as u8),
    )
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

#[inline]
fn cartesian_axis_edge_for_position(axis_position: AxisPosition) -> SharingGroupEdge {
    match axis_position {
        AxisPosition::Top | AxisPosition::Left => SharingGroupEdge::Start,
        AxisPosition::Bottom | AxisPosition::Right => SharingGroupEdge::End,
    }
}

#[inline]
fn cartesian_relevant_direction_for_axis(axis_position: AxisPosition) -> FacetDirection {
    match axis_position {
        AxisPosition::Top | AxisPosition::Bottom => FacetDirection::Row,
        AxisPosition::Left | AxisPosition::Right => FacetDirection::Column,
    }
}

fn project_levels_for_cartesian_axis(
    position_indices: &[usize],
    level_counts: &[usize],
    level_directions: &[FacetDirection],
    axis_position: AxisPosition,
) -> Option<(Vec<usize>, Vec<usize>, usize, SharingGroupEdge)> {
    if position_indices.len() != level_counts.len()
        || position_indices.len() != level_directions.len()
    {
        return None;
    }

    let edge = cartesian_axis_edge_for_position(axis_position);
    let relevant_direction = cartesian_relevant_direction_for_axis(axis_position);

    let mut projected_indices = Vec::with_capacity(position_indices.len());
    let mut projected_counts = Vec::with_capacity(level_counts.len());
    let mut relevant_depth = 0usize;

    // Keep orthogonal levels first so sharing groups are scoped within strips.
    for ((&index, &count), &direction) in position_indices
        .iter()
        .zip(level_counts.iter())
        .zip(level_directions.iter())
    {
        if direction != relevant_direction {
            projected_indices.push(index);
            projected_counts.push(count);
        }
    }

    // Append relevant levels last so sharing applies only within those levels.
    for ((&index, &count), &direction) in position_indices
        .iter()
        .zip(level_counts.iter())
        .zip(level_directions.iter())
    {
        if direction == relevant_direction {
            projected_indices.push(index);
            projected_counts.push(count);
            relevant_depth += 1;
        }
    }

    Some((projected_indices, projected_counts, relevant_depth, edge))
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
    sharing_level: SharingLevel,
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
        assert_eq!(domain_group_key(&path, SharingLevel::from_raw(0), 4), path);
        assert_eq!(
            domain_group_key(&path, SharingLevel::from_raw(1), 4),
            vec![s("A"), s("B"), s("C")]
        );
        assert_eq!(
            domain_group_key(&path, SharingLevel::from_raw(2), 4),
            vec![s("A"), s("B")]
        );
        assert!(domain_group_key(&path, SharingLevel::from_raw(4), 4).is_empty());
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
            SharingLevel::from_raw(1),
            FacetDirection::Column,
            AxisPosition::Left
        ));
        assert!(!show_axis_labels(
            &[0, 0, 1, 1],
            &counts,
            facet_depth,
            SharingLevel::from_raw(1),
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
            SharingLevel::GLOBAL,
            FacetDirection::Column,
            AxisPosition::Left
        ));
        assert!(!show_axis_labels(
            &[1],
            &counts,
            facet_depth,
            SharingLevel::GLOBAL,
            FacetDirection::Column,
            AxisPosition::Left
        ));

        assert!(!show_axis_labels(
            &[1],
            &counts,
            facet_depth,
            SharingLevel::GLOBAL,
            FacetDirection::Column,
            AxisPosition::Right
        ));
        assert!(show_axis_labels(
            &[2],
            &counts,
            facet_depth,
            SharingLevel::GLOBAL,
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
    fn cartesian_y_title_shared_per_row_strip() {
        let counts = vec![3, 2];
        let directions = vec![FacetDirection::Column, FacetDirection::Row];

        assert!(show_cartesian_axis_title(
            &[0, 0],
            &counts,
            &directions,
            AxisPosition::Left
        ));
        assert!(show_cartesian_axis_title(
            &[0, 1],
            &counts,
            &directions,
            AxisPosition::Left
        ));
        assert!(!show_cartesian_axis_title(
            &[1, 0],
            &counts,
            &directions,
            AxisPosition::Left
        ));
    }

    #[test]
    fn cartesian_x_bottom_shared_per_column_strip() {
        let counts = vec![3, 2];
        let directions = vec![FacetDirection::Column, FacetDirection::Row];

        assert!(!show_cartesian_axis_title(
            &[0, 0],
            &counts,
            &directions,
            AxisPosition::Bottom
        ));
        assert!(show_cartesian_axis_title(
            &[0, 1],
            &counts,
            &directions,
            AxisPosition::Bottom
        ));
        assert!(show_cartesian_axis_title(
            &[2, 1],
            &counts,
            &directions,
            AxisPosition::Bottom
        ));
    }

    #[test]
    fn cartesian_x_labels_sharing_clamps_to_relevant_depth() {
        let counts = vec![3, 2];
        let directions = vec![FacetDirection::Column, FacetDirection::Row];

        assert!(!show_cartesian_axis_labels(
            &[1, 0],
            &counts,
            &directions,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL
        ));
        assert!(show_cartesian_axis_labels(
            &[1, 1],
            &counts,
            &directions,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL
        ));
    }

    #[test]
    fn cartesian_labels_free_show_every_subplot() {
        let counts = vec![3, 2];
        let directions = vec![FacetDirection::Column, FacetDirection::Row];

        assert!(show_cartesian_axis_labels(
            &[1, 0],
            &counts,
            &directions,
            AxisPosition::Bottom,
            SharingLevel::FREE
        ));
        assert!(show_cartesian_axis_labels(
            &[1, 1],
            &counts,
            &directions,
            AxisPosition::Bottom,
            SharingLevel::FREE
        ));
    }

    #[test]
    fn cartesian_axis_without_relevant_levels_stays_visible() {
        let counts = vec![3, 2];
        let directions = vec![FacetDirection::Column, FacetDirection::Column];

        assert!(show_cartesian_axis_labels(
            &[2, 1],
            &counts,
            &directions,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL
        ));
        assert!(show_cartesian_axis_title(
            &[2, 1],
            &counts,
            &directions,
            AxisPosition::Bottom
        ));
    }

    #[test]
    fn domain_group_key_supports_null_values() {
        let path = vec![ScalarValue::Null, s("B"), ScalarValue::Int64(Some(5))];
        let key = domain_group_key(&path, SharingLevel::from_raw(1), 3);
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
            SharingLevel::from_raw(1),
            LegendPosition::Top
        ));
        assert!(legend_owner_for_position(
            &idx_end,
            &counts,
            facet_depth,
            SharingLevel::from_raw(1),
            LegendPosition::Bottom
        ));
        assert!(legend_owner_for_position(
            &idx_start,
            &counts,
            facet_depth,
            SharingLevel::from_raw(1),
            LegendPosition::Left
        ));
        assert!(legend_owner_for_position(
            &idx_end,
            &counts,
            facet_depth,
            SharingLevel::from_raw(1),
            LegendPosition::Right
        ));
    }
}
