//! Generic sharing math for child-frame containers.
//!
//! This module owns the mode-neutral pieces of sharing behavior: sharing-level
//! normalization, path grouping, and start/end edge ownership. Facets add
//! semantic roles on top of these primitives, while concat and future
//! child-frame containers can reuse the same grouping rules.

use datafusion::common::ScalarValue;

use crate::chart_core::{CoordinationAxis, SharingLevel};

use super::{CoordinationKind, CoordinationScopeKey};

/// Compute a shared-group key from a full child-frame path.
///
/// `path` is expected to contain one value per path-depth level.
/// `sharing_level` is interpreted as:
/// - `0`: no sharing (keep full path),
/// - `N`: remove last `N` path components,
/// - `>= path_depth`: global sharing (empty key).
pub(crate) fn shared_path_key(
    path: &[ScalarValue],
    sharing_level: SharingLevel,
    path_depth: u8,
) -> Vec<ScalarValue> {
    debug_assert_eq!(
        path_depth as usize,
        path.len(),
        "path_depth ({}) must equal path.len() ({})",
        path_depth,
        path.len()
    );

    if sharing_level >= path_depth {
        vec![]
    } else {
        let keep_count = sharing_level.ancestor_keep_count(path.len(), path_depth);
        path.iter().take(keep_count).cloned().collect()
    }
}

/// Compute the ancestor path to use for Level(N) value enumeration.
///
/// `parent_path` is the path to parent groups of the child level and has length
/// `child_depth - 1`.
pub(crate) fn enumeration_ancestor_path(
    parent_path: &[ScalarValue],
    sharing_level: SharingLevel,
    child_depth: u8,
) -> Vec<ScalarValue> {
    if sharing_level.is_free() {
        return parent_path.to_vec();
    }

    let parent_depth = (child_depth as usize).saturating_sub(1);
    if sharing_level.raw() as usize >= parent_depth {
        vec![]
    } else {
        let keep_count = parent_depth.saturating_sub(sharing_level.raw() as usize);
        parent_path.iter().take(keep_count).cloned().collect()
    }
}

/// Compute the suffix boundary for sharing-group checks.
///
/// With path depth `D` and sharing level `N`, sharing groups are defined by the
/// prefix of length `D - N`; first/last checks are performed over the suffix
/// `[D - N .. D)`.
pub(crate) fn sharing_group_boundary(path_depth: u8, sharing_level: SharingLevel) -> usize {
    sharing_level.group_boundary(path_depth)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SharingGroupEdge {
    Start,
    End,
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

/// One semantic ownership group plus the edge rule that picks its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EdgeOwnershipScope {
    pub(crate) key: CoordinationScopeKey,
    pub(crate) edge: SharingGroupEdge,
    pub(crate) position_indices: Vec<usize>,
    pub(crate) level_counts: Vec<usize>,
    pub(crate) boundary: usize,
}

impl EdgeOwnershipScope {
    pub(crate) fn current_position_owns(&self) -> bool {
        owner_for_edge(
            self.edge,
            &self.position_indices,
            &self.level_counts,
            self.boundary,
        )
    }
}

pub(crate) fn owner_for_scope(scope: Option<EdgeOwnershipScope>) -> bool {
    scope
        .as_ref()
        .map(EdgeOwnershipScope::current_position_owns)
        .unwrap_or(true)
}

/// Generic request for a shared-edge ownership scope.
pub(crate) struct EdgeOwnershipRequest<'a> {
    pub(crate) kind: CoordinationKind,
    pub(crate) channel: String,
    pub(crate) edge: SharingGroupEdge,
    pub(crate) position_indices: &'a [usize],
    pub(crate) level_counts: &'a [usize],
    pub(crate) path_depth: u8,
    pub(crate) sharing_level: SharingLevel,
}

impl<'a> EdgeOwnershipRequest<'a> {
    pub(crate) fn new(
        kind: CoordinationKind,
        channel: impl Into<String>,
        edge: SharingGroupEdge,
        position_indices: &'a [usize],
        level_counts: &'a [usize],
        path_depth: u8,
        sharing_level: SharingLevel,
    ) -> Self {
        Self {
            kind,
            channel: channel.into(),
            edge,
            position_indices,
            level_counts,
            path_depth,
            sharing_level,
        }
    }

    pub(crate) fn from_projection(
        kind: CoordinationKind,
        channel: impl Into<String>,
        projection: &'a ContainerEdgeLevelProjection,
        sharing_level: SharingLevel,
    ) -> Self {
        Self::new(
            kind,
            channel,
            projection.edge,
            &projection.position_indices,
            &projection.level_counts,
            projection.path_depth(),
            sharing_level,
        )
    }
}

pub(crate) fn edge_ownership_scope_for_request(
    request: EdgeOwnershipRequest<'_>,
) -> EdgeOwnershipScope {
    let boundary = sharing_group_boundary(request.path_depth, request.sharing_level);
    let group_path = request
        .position_indices
        .get(..boundary.min(request.position_indices.len()))
        .unwrap_or(request.position_indices)
        .to_vec();

    EdgeOwnershipScope {
        key: CoordinationScopeKey::position_path(request.kind, group_path)
            .with_channel(request.channel),
        edge: request.edge,
        position_indices: request.position_indices.to_vec(),
        level_counts: request.level_counts.to_vec(),
        boundary,
    }
}

/// Projection of nested child-frame levels for one shared edge.
///
/// Orthogonal levels are kept as a prefix so sharing stays scoped to each
/// strip. Levels along the relevant direction are appended as the suffix where
/// the requested `SharingLevel` applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContainerEdgeLevelProjection {
    pub(crate) position_indices: Vec<usize>,
    pub(crate) level_counts: Vec<usize>,
    pub(crate) relevant_depth: usize,
    pub(crate) edge: SharingGroupEdge,
}

impl ContainerEdgeLevelProjection {
    #[inline]
    pub(crate) fn path_depth(&self) -> u8 {
        self.position_indices.len() as u8
    }
}

pub(crate) fn project_container_edge_levels(
    position_indices: &[usize],
    level_counts: &[usize],
    level_axes: &[CoordinationAxis],
    relevant_axis: CoordinationAxis,
    edge: SharingGroupEdge,
) -> Option<ContainerEdgeLevelProjection> {
    if position_indices.len() != level_counts.len() || position_indices.len() != level_axes.len() {
        return None;
    }

    let mut projected_indices = Vec::with_capacity(position_indices.len());
    let mut projected_counts = Vec::with_capacity(level_counts.len());
    let mut relevant_depth = 0usize;

    for ((&index, &count), &axis) in position_indices
        .iter()
        .zip(level_counts.iter())
        .zip(level_axes.iter())
    {
        if axis != relevant_axis {
            projected_indices.push(index);
            projected_counts.push(count);
        }
    }

    for ((&index, &count), &axis) in position_indices
        .iter()
        .zip(level_counts.iter())
        .zip(level_axes.iter())
    {
        if axis == relevant_axis {
            projected_indices.push(index);
            projected_counts.push(count);
            relevant_depth += 1;
        }
    }

    Some(ContainerEdgeLevelProjection {
        position_indices: projected_indices,
        level_counts: projected_counts,
        relevant_depth,
        edge,
    })
}

#[cfg(test)]
mod tests {
    use crate::chart_core::ScaleSharing;

    use super::*;

    fn s(v: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(v.to_string()))
    }

    #[test]
    fn sharing_level_raw_roundtrip() {
        for raw in [0_u8, 1, 7, 254, 255] {
            let level = SharingLevel::from_raw(raw);
            assert_eq!(level.raw(), raw);
            assert_eq!(u8::from(level), raw);
        }
    }

    #[test]
    fn sharing_level_free_and_global_detection() {
        assert!(SharingLevel::FREE.is_free());
        assert!(!SharingLevel::FREE.is_global());
        assert!(SharingLevel::GLOBAL.is_global());
        assert!(!SharingLevel::GLOBAL.is_free());
    }

    #[test]
    fn sharing_level_clamp_to_depth() {
        assert_eq!(SharingLevel::from_raw(0).clamp_to_depth(3).raw(), 0);
        assert_eq!(SharingLevel::from_raw(2).clamp_to_depth(3).raw(), 2);
        assert_eq!(SharingLevel::from_raw(5).clamp_to_depth(3).raw(), 3);
        assert_eq!(SharingLevel::GLOBAL.clamp_to_depth(4).raw(), 4);
    }

    #[test]
    fn sharing_level_scale_sharing_conversions_normalized() {
        assert_eq!(
            SharingLevel::from(ScaleSharing::Free),
            SharingLevel::from_raw(0)
        );
        assert_eq!(
            SharingLevel::from(ScaleSharing::Shared),
            SharingLevel::GLOBAL
        );
        assert_eq!(
            SharingLevel::from(ScaleSharing::Level(3)),
            SharingLevel::from_raw(3)
        );

        let back_to_scale: ScaleSharing = SharingLevel::GLOBAL.into();
        assert_eq!(back_to_scale, ScaleSharing::Level(255));
    }

    #[test]
    fn shared_path_key_level0_keeps_full_path() {
        let path = vec![s("A"), s("B"), s("C")];
        assert_eq!(
            shared_path_key(&path, SharingLevel::from_raw(0), path.len() as u8),
            path
        );
    }

    #[test]
    fn shared_path_key_level2_removes_two() {
        let path = vec![s("A"), s("B"), s("C"), s("D")];
        assert_eq!(
            shared_path_key(&path, SharingLevel::from_raw(2), path.len() as u8),
            vec![s("A"), s("B")]
        );
    }

    #[test]
    fn shared_path_key_global_sharing_is_empty() {
        let path = vec![s("A"), s("B"), s("C")];
        assert!(
            shared_path_key(
                &path,
                SharingLevel::from_raw(path.len() as u8),
                path.len() as u8
            )
            .is_empty()
        );
    }

    #[test]
    fn enumeration_ancestor_level0_keeps_parent_path() {
        let parent_path = vec![s("Div"), s("Dept")];
        assert_eq!(
            enumeration_ancestor_path(&parent_path, SharingLevel::from_raw(0), 3),
            parent_path
        );
    }

    #[test]
    fn enumeration_ancestor_level1_drops_one_parent_level() {
        let parent_path = vec![s("Div"), s("Dept")];
        assert_eq!(
            enumeration_ancestor_path(&parent_path, SharingLevel::from_raw(1), 3),
            vec![s("Div")]
        );
    }

    #[test]
    fn enumeration_ancestor_global_uses_root() {
        let parent_path = vec![s("Div"), s("Dept")];
        assert!(enumeration_ancestor_path(&parent_path, SharingLevel::from_raw(2), 3).is_empty());
        assert!(enumeration_ancestor_path(&parent_path, SharingLevel::from_raw(255), 3).is_empty());
    }

    #[test]
    fn sharing_group_boundary_examples() {
        assert_eq!(sharing_group_boundary(4, SharingLevel::from_raw(0)), 4);
        assert_eq!(sharing_group_boundary(4, SharingLevel::from_raw(1)), 3);
        assert_eq!(sharing_group_boundary(4, SharingLevel::from_raw(2)), 2);
        assert_eq!(sharing_group_boundary(4, SharingLevel::from_raw(4)), 0);
        assert_eq!(sharing_group_boundary(4, SharingLevel::from_raw(255)), 0);
    }

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

    #[test]
    fn edge_ownership_request_groups_by_sharing_prefix_and_role() {
        let owner = edge_ownership_scope_for_request(EdgeOwnershipRequest::new(
            CoordinationKind::GuideOwnership,
            "axis:x",
            SharingGroupEdge::End,
            &[1, 0, 1],
            &[2, 2, 2],
            3,
            SharingLevel::from_raw(1),
        ));
        let peer = edge_ownership_scope_for_request(EdgeOwnershipRequest::new(
            CoordinationKind::GuideOwnership,
            "axis:x",
            SharingGroupEdge::End,
            &[1, 0, 0],
            &[2, 2, 2],
            3,
            SharingLevel::from_raw(1),
        ));
        let other_role = edge_ownership_scope_for_request(EdgeOwnershipRequest::new(
            CoordinationKind::GuideOwnership,
            "axis:y",
            SharingGroupEdge::End,
            &[1, 0, 1],
            &[2, 2, 2],
            3,
            SharingLevel::from_raw(1),
        ));

        assert_eq!(owner.key, peer.key);
        assert_ne!(owner.key, other_role.key);
        assert!(owner.current_position_owns());
        assert!(!peer.current_position_owns());
    }

    #[test]
    fn project_container_edge_levels_keeps_orthogonal_levels_as_prefix() {
        let projection = project_container_edge_levels(
            &[2, 1],
            &[3, 2],
            &[CoordinationAxis::Horizontal, CoordinationAxis::Vertical],
            CoordinationAxis::Vertical,
            SharingGroupEdge::End,
        )
        .expect("projection should be valid");

        assert_eq!(projection.position_indices, vec![2, 1]);
        assert_eq!(projection.level_counts, vec![3, 2]);
        assert_eq!(projection.relevant_depth, 1);
        assert_eq!(projection.edge, SharingGroupEdge::End);
    }

    #[test]
    fn project_container_edge_levels_moves_relevant_levels_to_suffix() {
        let projection = project_container_edge_levels(
            &[0, 1, 2],
            &[2, 3, 4],
            &[
                CoordinationAxis::Vertical,
                CoordinationAxis::Horizontal,
                CoordinationAxis::Vertical,
            ],
            CoordinationAxis::Vertical,
            SharingGroupEdge::Start,
        )
        .expect("projection should be valid");

        assert_eq!(projection.position_indices, vec![1, 0, 2]);
        assert_eq!(projection.level_counts, vec![3, 2, 4]);
        assert_eq!(projection.relevant_depth, 2);
        assert_eq!(projection.path_depth(), 3);
    }

    #[test]
    fn projection_can_drive_edge_ownership_scope() {
        let projection = project_container_edge_levels(
            &[0, 1],
            &[3, 2],
            &[CoordinationAxis::Horizontal, CoordinationAxis::Vertical],
            CoordinationAxis::Vertical,
            SharingGroupEdge::End,
        )
        .expect("projection should be valid");

        let owner = edge_ownership_scope_for_request(EdgeOwnershipRequest::from_projection(
            CoordinationKind::GuideOwnership,
            "x:bottom",
            &projection,
            SharingLevel::from_raw(projection.relevant_depth as u8),
        ));

        assert_eq!(owner.boundary, 1);
        assert!(owner.current_position_owns());
    }

    #[test]
    fn project_container_edge_levels_rejects_mismatched_lengths() {
        assert!(
            project_container_edge_levels(
                &[0, 1],
                &[2],
                &[CoordinationAxis::Horizontal, CoordinationAxis::Vertical],
                CoordinationAxis::Vertical,
                SharingGroupEdge::End,
            )
            .is_none()
        );
    }
}
