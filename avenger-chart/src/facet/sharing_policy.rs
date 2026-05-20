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
    plot::compiled::{CoordinationKind, CoordinationScopeKey},
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuideOwnershipRole {
    FacetAxisLabels,
    FacetAxisTitle,
    CartesianAxisLabels,
    CartesianAxisTitle,
}

/// One semantic guide-ownership group plus the edge rule that picks its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GuideOwnershipScope {
    pub(crate) key: CoordinationScopeKey,
    pub(crate) edge: SharingGroupEdge,
    pub(crate) position_indices: Vec<usize>,
    pub(crate) level_counts: Vec<usize>,
    pub(crate) boundary: usize,
}

impl GuideOwnershipScope {
    pub(crate) fn current_position_owns(&self) -> bool {
        sharing_kernel::owner_for_edge(
            self.edge,
            &self.position_indices,
            &self.level_counts,
            self.boundary,
        )
    }
}

pub(crate) fn guide_owner_for_scope(scope: Option<GuideOwnershipScope>) -> bool {
    scope
        .as_ref()
        .map(GuideOwnershipScope::current_position_owns)
        .unwrap_or(true)
}

fn guide_ownership_scope(
    role: GuideOwnershipRole,
    axis_position: AxisPosition,
    edge: SharingGroupEdge,
    position_indices: &[usize],
    level_counts: &[usize],
    facet_depth: u8,
    sharing_level: SharingLevel,
) -> GuideOwnershipScope {
    let boundary = sharing_kernel::group_boundary(facet_depth, sharing_level);
    let group_path = position_indices
        .get(..boundary.min(position_indices.len()))
        .unwrap_or(position_indices)
        .to_vec();
    let channel = format!("{role:?}:{axis_position:?}");

    GuideOwnershipScope {
        key: CoordinationScopeKey::position_path(CoordinationKind::GuideOwnership, group_path)
            .with_channel(channel),
        edge,
        position_indices: position_indices.to_vec(),
        level_counts: level_counts.to_vec(),
        boundary,
    }
}

pub(crate) fn axis_label_ownership_scope(
    position_indices: &[usize],
    level_counts: &[usize],
    facet_depth: u8,
    sharing_level: SharingLevel,
    direction: FacetDirection,
    axis_position: AxisPosition,
) -> Option<GuideOwnershipScope> {
    axis_edge_for_position(direction, axis_position).map(|edge| {
        guide_ownership_scope(
            GuideOwnershipRole::FacetAxisLabels,
            axis_position,
            edge,
            position_indices,
            level_counts,
            facet_depth,
            sharing_level,
        )
    })
}

pub(crate) fn axis_title_ownership_scope(
    position_indices: &[usize],
    level_counts: &[usize],
    facet_depth: u8,
    direction: FacetDirection,
    axis_position: AxisPosition,
) -> Option<GuideOwnershipScope> {
    axis_edge_for_position(direction, axis_position).map(|edge| {
        guide_ownership_scope(
            GuideOwnershipRole::FacetAxisTitle,
            axis_position,
            edge,
            position_indices,
            level_counts,
            facet_depth,
            SharingLevel::GLOBAL,
        )
    })
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
    guide_owner_for_scope(axis_label_ownership_scope(
        position_indices,
        level_counts,
        facet_depth,
        sharing_level,
        direction,
        axis_position,
    ))
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
    guide_owner_for_scope(axis_title_ownership_scope(
        position_indices,
        level_counts,
        facet_depth,
        direction,
        axis_position,
    ))
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
    guide_owner_for_scope(cartesian_axis_label_ownership_scope(
        position_indices,
        level_counts,
        level_directions,
        axis_position,
        sharing_level,
    ))
}

pub(crate) fn cartesian_axis_label_ownership_scope(
    position_indices: &[usize],
    level_counts: &[usize],
    level_directions: &[FacetDirection],
    axis_position: AxisPosition,
    sharing_level: SharingLevel,
) -> Option<GuideOwnershipScope> {
    cartesian_axis_ownership_scope_for_sharing(
        GuideOwnershipRole::CartesianAxisLabels,
        position_indices,
        level_counts,
        level_directions,
        axis_position,
        sharing_level,
    )
}

pub(crate) fn cartesian_axis_ownership_scope_for_sharing(
    role: GuideOwnershipRole,
    position_indices: &[usize],
    level_counts: &[usize],
    level_directions: &[FacetDirection],
    axis_position: AxisPosition,
    sharing_level: SharingLevel,
) -> Option<GuideOwnershipScope> {
    let Some((projected_indices, projected_counts, relevant_depth, edge)) =
        project_levels_for_cartesian_axis(
            position_indices,
            level_counts,
            level_directions,
            axis_position,
        )
    else {
        return None;
    };

    if relevant_depth == 0 {
        return None;
    }

    let labels_sharing = sharing_level.clamp_to_depth(relevant_depth as u8);
    Some(guide_ownership_scope(
        role,
        axis_position,
        edge,
        &projected_indices,
        &projected_counts,
        projected_indices.len() as u8,
        labels_sharing,
    ))
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
    guide_owner_for_scope(cartesian_axis_title_ownership_scope(
        position_indices,
        level_counts,
        level_directions,
        axis_position,
    ))
}

pub(crate) fn cartesian_axis_title_ownership_scope(
    position_indices: &[usize],
    level_counts: &[usize],
    level_directions: &[FacetDirection],
    axis_position: AxisPosition,
) -> Option<GuideOwnershipScope> {
    let relevant_depth = level_directions
        .iter()
        .filter(|&&direction| direction == cartesian_relevant_direction_for_axis(axis_position))
        .count();
    cartesian_axis_ownership_scope_for_sharing(
        GuideOwnershipRole::CartesianAxisTitle,
        position_indices,
        level_counts,
        level_directions,
        axis_position,
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
    fn facet_axis_label_ownership_scope_groups_by_sharing_prefix() {
        let counts = vec![2, 2, 2, 2];
        let facet_depth = 4;
        let owner = axis_label_ownership_scope(
            &[0, 0, 1, 0],
            &counts,
            facet_depth,
            SharingLevel::from_raw(1),
            FacetDirection::Column,
            AxisPosition::Left,
        )
        .unwrap();
        let peer = axis_label_ownership_scope(
            &[0, 0, 1, 1],
            &counts,
            facet_depth,
            SharingLevel::from_raw(1),
            FacetDirection::Column,
            AxisPosition::Left,
        )
        .unwrap();

        assert_eq!(owner.key, peer.key);
        assert_eq!(owner.key.kind, CoordinationKind::GuideOwnership);
        assert!(owner.current_position_owns());
        assert!(!peer.current_position_owns());
    }

    #[test]
    fn facet_axis_title_ownership_scope_uses_global_group() {
        let counts = vec![2, 2];
        let owner = axis_title_ownership_scope(
            &[0, 0],
            &counts,
            2,
            FacetDirection::Column,
            AxisPosition::Left,
        )
        .unwrap();
        let peer = axis_title_ownership_scope(
            &[0, 1],
            &counts,
            2,
            FacetDirection::Column,
            AxisPosition::Left,
        )
        .unwrap();

        assert_eq!(owner.key, peer.key);
        assert_eq!(owner.boundary, 0);
        assert!(owner.current_position_owns());
        assert!(!peer.current_position_owns());
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
    fn cartesian_axis_ownership_scope_preserves_orthogonal_strips() {
        let counts = vec![3, 2];
        let directions = vec![FacetDirection::Column, FacetDirection::Row];

        let top_of_first_column = cartesian_axis_label_ownership_scope(
            &[0, 0],
            &counts,
            &directions,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL,
        )
        .unwrap();
        let bottom_of_first_column = cartesian_axis_label_ownership_scope(
            &[0, 1],
            &counts,
            &directions,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL,
        )
        .unwrap();
        let bottom_of_other_column = cartesian_axis_label_ownership_scope(
            &[2, 1],
            &counts,
            &directions,
            AxisPosition::Bottom,
            SharingLevel::GLOBAL,
        )
        .unwrap();

        assert_eq!(top_of_first_column.key, bottom_of_first_column.key);
        assert_ne!(bottom_of_first_column.key, bottom_of_other_column.key);
        assert!(!top_of_first_column.current_position_owns());
        assert!(bottom_of_first_column.current_position_owns());
        assert!(bottom_of_other_column.current_position_owns());
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
