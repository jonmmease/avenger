//! Mode-neutral sharing math for child-frame containers.
//!
//! These helpers operate on paths, position/count vectors, and coordination
//! axes only. They do not know about facet trees, concat layout, or concrete
//! child-frame runtime state.

use datafusion::common::ScalarValue;

use crate::{CoordinationAxis, SharingLevel};

/// Compute a shared-group key from a full child-frame path.
///
/// `path` is expected to contain one value per path-depth level.
/// `sharing_level` is interpreted as:
/// - `0`: no sharing (keep full path),
/// - `N`: remove last `N` path components,
/// - `>= path_depth`: global sharing (empty key).
#[doc(hidden)]
pub fn shared_path_key(
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
#[doc(hidden)]
pub fn enumeration_ancestor_path(
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
#[doc(hidden)]
pub fn sharing_group_boundary(path_depth: u8, sharing_level: SharingLevel) -> usize {
    sharing_level.group_boundary(path_depth)
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharingGroupEdge {
    Start,
    End,
}

#[doc(hidden)]
#[inline]
pub fn is_group_start(position_indices: &[usize], boundary: usize) -> bool {
    position_indices
        .get(boundary..)
        .map(|suffix| suffix.iter().all(|&i| i == 0))
        .unwrap_or(true)
}

#[doc(hidden)]
#[inline]
pub fn is_group_end(position_indices: &[usize], level_counts: &[usize], boundary: usize) -> bool {
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

#[doc(hidden)]
#[inline]
pub fn owner_for_edge(
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

/// Projection of nested child-frame levels for one shared edge.
///
/// Orthogonal levels are kept as a prefix so sharing stays scoped to each
/// strip. Levels along the relevant direction are appended as the suffix where
/// the requested `SharingLevel` applies.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerEdgeLevelProjection {
    pub position_indices: Vec<usize>,
    pub level_counts: Vec<usize>,
    pub relevant_depth: usize,
    pub edge: SharingGroupEdge,
}

impl ContainerEdgeLevelProjection {
    #[inline]
    pub fn path_depth(&self) -> u8 {
        self.position_indices.len() as u8
    }
}

#[doc(hidden)]
pub fn project_container_edge_levels(
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
    use super::*;
    use crate::Sharing;

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
    fn sharing_level_conversions_normalized() {
        assert_eq!(SharingLevel::from(Sharing::Free), SharingLevel::from_raw(0));
        assert_eq!(SharingLevel::from(Sharing::Shared), SharingLevel::GLOBAL);
        assert_eq!(
            SharingLevel::from(Sharing::Level(3)),
            SharingLevel::from_raw(3)
        );

        let back_to_scale: Sharing = SharingLevel::GLOBAL.into();
        assert_eq!(back_to_scale, Sharing::Level(255));
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
