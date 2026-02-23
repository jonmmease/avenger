//! Canonical path and sharing-level math for faceting.
//!
//! This module centralizes the path transforms used by facet layout, enumeration,
//! visibility grouping, and domain coordination so all call sites share the same
//! semantics.

use datafusion::common::ScalarValue;

use crate::facet::sharing_level::SharingLevel;

/// Compute an ancestor key from a full cell path for Level(N) sharing.
///
/// `full_cell_path` is expected to contain one value per facet depth level.
/// `sharing_level` is interpreted as:
/// - `0`: no sharing (keep full path),
/// - `N`: remove last `N` path components,
/// - `>= facet_depth`: global sharing (empty key).
pub(crate) fn ancestor_key(
    full_cell_path: &[ScalarValue],
    sharing_level: SharingLevel,
    facet_depth: u8,
) -> Vec<ScalarValue> {
    debug_assert_eq!(
        facet_depth as usize,
        full_cell_path.len(),
        "facet_depth ({}) must equal full_cell_path.len() ({})",
        facet_depth,
        full_cell_path.len()
    );

    if sharing_level >= facet_depth {
        vec![]
    } else {
        let keep_count = sharing_level.ancestor_keep_count(full_cell_path.len(), facet_depth);
        full_cell_path.iter().take(keep_count).cloned().collect()
    }
}

/// Compute the ancestor path to use for Level(N) value enumeration at a facet node.
///
/// `facet_path` is the path to parent groups of the current facet level and has
/// length `facet_depth - 1`.
///
/// Semantics:
/// - `0`: keep local parent path (enumerate current subtree),
/// - `>= facet_depth - 1`: use root-level enumeration,
/// - otherwise: drop last `sharing_level` components from `facet_path`.
pub(crate) fn enumeration_ancestor_path(
    facet_path: &[ScalarValue],
    sharing_level: SharingLevel,
    facet_depth: u8,
) -> Vec<ScalarValue> {
    if sharing_level.is_free() {
        return facet_path.to_vec();
    }

    let parent_depth = (facet_depth as usize).saturating_sub(1);
    if sharing_level.raw() as usize >= parent_depth {
        vec![]
    } else {
        let keep_count = parent_depth.saturating_sub(sharing_level.raw() as usize);
        facet_path.iter().take(keep_count).cloned().collect()
    }
}

/// Compute the suffix boundary for sharing-group checks.
///
/// With facet depth `D` and sharing level `N`, sharing groups are defined by the
/// prefix of length `D - N`; first/last checks are performed over the suffix
/// `[D - N .. D)`.
pub(crate) fn sharing_group_boundary(facet_depth: u8, sharing_level: SharingLevel) -> usize {
    sharing_level.group_boundary(facet_depth)
}

/// Compute ancestor key for nested facet measurement sharing.
///
/// Nested sharing level is defined one level deeper than the current cell path.
/// For a current path at depth `D`, nested facet depth is `D + 1`, and Level(N)
/// at nested depth translates to removing `N - 1` components from current path.
pub(crate) fn nested_measurement_ancestor_key(
    current_cell_path: &[ScalarValue],
    nested_sharing_level: SharingLevel,
    nested_depth: u8,
) -> Vec<ScalarValue> {
    debug_assert_eq!(
        nested_depth as usize,
        current_cell_path.len() + 1,
        "nested_depth ({}) must equal current_cell_path.len()+1 ({})",
        nested_depth,
        current_cell_path.len() + 1
    );

    let translated_level = SharingLevel::from_raw(nested_sharing_level.raw().saturating_sub(1));
    ancestor_key(
        current_cell_path,
        translated_level,
        current_cell_path.len() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(v.to_string()))
    }

    #[test]
    fn ancestor_key_level0_keeps_full_path() {
        let path = vec![s("A"), s("B"), s("C")];
        assert_eq!(
            ancestor_key(&path, SharingLevel::from_raw(0), path.len() as u8),
            path
        );
    }

    #[test]
    fn ancestor_key_level2_removes_two() {
        let path = vec![s("A"), s("B"), s("C"), s("D")];
        assert_eq!(
            ancestor_key(&path, SharingLevel::from_raw(2), path.len() as u8),
            vec![s("A"), s("B")]
        );
    }

    #[test]
    fn ancestor_key_global_sharing_is_empty() {
        let path = vec![s("A"), s("B"), s("C")];
        assert!(
            ancestor_key(
                &path,
                SharingLevel::from_raw(path.len() as u8),
                path.len() as u8
            )
            .is_empty()
        );
    }

    #[test]
    fn enumeration_ancestor_level0_keeps_facet_path() {
        let facet_path = vec![s("Div"), s("Dept")];
        assert_eq!(
            enumeration_ancestor_path(&facet_path, SharingLevel::from_raw(0), 3),
            facet_path
        );
    }

    #[test]
    fn enumeration_ancestor_level1_drops_one_parent_level() {
        let facet_path = vec![s("Div"), s("Dept")];
        assert_eq!(
            enumeration_ancestor_path(&facet_path, SharingLevel::from_raw(1), 3),
            vec![s("Div")]
        );
    }

    #[test]
    fn enumeration_ancestor_global_uses_root() {
        let facet_path = vec![s("Div"), s("Dept")];
        assert!(enumeration_ancestor_path(&facet_path, SharingLevel::from_raw(2), 3).is_empty());
        assert!(enumeration_ancestor_path(&facet_path, SharingLevel::from_raw(255), 3).is_empty());
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
    fn nested_measurement_ancestor_translates_level0_to_full_path() {
        let path = vec![s("Div"), s("Dept"), s("Team")];
        let key = nested_measurement_ancestor_key(&path, SharingLevel::from_raw(0), 4);
        assert_eq!(key, path);
    }

    #[test]
    fn nested_measurement_ancestor_translates_level1_to_full_path() {
        let path = vec![s("Div"), s("Dept"), s("Team")];
        let key = nested_measurement_ancestor_key(&path, SharingLevel::from_raw(1), 4);
        assert_eq!(key, path);
    }

    #[test]
    fn nested_measurement_ancestor_translates_level2_to_drop_one() {
        let path = vec![s("Div"), s("Dept"), s("Team")];
        let key = nested_measurement_ancestor_key(&path, SharingLevel::from_raw(2), 4);
        assert_eq!(key, vec![s("Div"), s("Dept")]);
    }

    #[test]
    fn nested_measurement_ancestor_shared_like_levels_map_to_global() {
        let path = vec![s("Div"), s("Dept"), s("Team")];
        assert!(nested_measurement_ancestor_key(&path, SharingLevel::from_raw(4), 4).is_empty());
        assert!(nested_measurement_ancestor_key(&path, SharingLevel::from_raw(255), 4).is_empty());
    }
}
