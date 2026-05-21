//! Generic sharing math for child-frame containers.
//!
//! This module owns the mode-neutral pieces of sharing behavior: sharing-level
//! normalization, path grouping, and start/end edge ownership. Facets add
//! semantic roles on top of these primitives, while concat and future
//! child-frame containers can reuse the same grouping rules.

use datafusion::common::ScalarValue;

use crate::channel::config_traits::ScaleSharing;

use super::{CoordinationKind, CoordinationScopeKey};

/// Canonical internal representation for child-frame container sharing.
///
/// Semantics:
/// - `0` => Free
/// - `N` => Level(N)
/// - `255` => Global (fully shared)
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SharingLevel(u8);

impl SharingLevel {
    pub(crate) const FREE: Self = Self(0);
    pub(crate) const GLOBAL: Self = Self(u8::MAX);

    #[inline]
    pub(crate) fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    #[inline]
    pub(crate) fn raw(self) -> u8 {
        self.0
    }

    #[inline]
    pub(crate) fn is_free(self) -> bool {
        self.0 == Self::FREE.0
    }

    #[inline]
    pub(crate) fn is_global(self) -> bool {
        self.0 == Self::GLOBAL.0
    }

    #[inline]
    pub(crate) fn clamp_to_depth(self, depth: u8) -> Self {
        Self(self.0.min(depth))
    }

    #[inline]
    pub(crate) fn group_boundary(self, path_depth: u8) -> usize {
        (path_depth as usize).saturating_sub(self.0 as usize)
    }

    #[inline]
    pub(crate) fn ancestor_keep_count(self, path_len: usize, path_depth: u8) -> usize {
        let effective = self.clamp_to_depth(path_depth);
        path_len.saturating_sub(effective.0 as usize)
    }
}

impl From<u8> for SharingLevel {
    fn from(value: u8) -> Self {
        Self::from_raw(value)
    }
}

impl From<SharingLevel> for u8 {
    fn from(value: SharingLevel) -> Self {
        value.raw()
    }
}

impl From<ScaleSharing> for SharingLevel {
    fn from(value: ScaleSharing) -> Self {
        Self::from_raw(value.to_level())
    }
}

impl From<SharingLevel> for ScaleSharing {
    fn from(value: SharingLevel) -> Self {
        ScaleSharing::from_level(value.raw())
    }
}

impl PartialEq<u8> for SharingLevel {
    fn eq(&self, other: &u8) -> bool {
        self.0 == *other
    }
}

impl PartialOrd<u8> for SharingLevel {
    fn partial_cmp(&self, other: &u8) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(other)
    }
}

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

pub(crate) fn edge_ownership_scope(
    kind: CoordinationKind,
    channel: impl Into<String>,
    edge: SharingGroupEdge,
    position_indices: &[usize],
    level_counts: &[usize],
    path_depth: u8,
    sharing_level: SharingLevel,
) -> EdgeOwnershipScope {
    let boundary = sharing_group_boundary(path_depth, sharing_level);
    let group_path = position_indices
        .get(..boundary.min(position_indices.len()))
        .unwrap_or(position_indices)
        .to_vec();

    EdgeOwnershipScope {
        key: CoordinationScopeKey::position_path(kind, group_path).with_channel(channel.into()),
        edge,
        position_indices: position_indices.to_vec(),
        level_counts: level_counts.to_vec(),
        boundary,
    }
}

#[cfg(test)]
mod tests {
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
}
