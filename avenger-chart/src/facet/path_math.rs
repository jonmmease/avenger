//! Facet-specific sharing path translation helpers.
//!
//! Generic sharing-level path grouping lives in
//! `plot::compiled::container_sharing`. This module keeps the facet-recursion
//! translation that maps child facet slot-sharing semantics onto the current
//! cell path.

#[cfg(test)]
use datafusion::common::ScalarValue;

#[cfg(test)]
use avenger_chart_core::SharingLevel;

#[cfg(test)]
use crate::plot::compiled::shared_path_key;

/// Compute the current-cell ancestor key implied by a child facet's slot sharing.
///
/// Child facet slot sharing is expressed one level deeper than the current cell
/// path. For a current path at depth `D`, child facet depth is `D + 1`, and
/// `Level(N)` at child depth translates to removing `N - 1` components from the
/// current path.
#[cfg(test)]
pub(crate) fn child_facet_slot_ancestor_key(
    current_cell_path: &[ScalarValue],
    child_facet_slot_sharing: SharingLevel,
    child_facet_depth: u8,
) -> Vec<ScalarValue> {
    debug_assert_eq!(
        child_facet_depth as usize,
        current_cell_path.len() + 1,
        "child_facet_depth ({}) must equal current_cell_path.len()+1 ({})",
        child_facet_depth,
        current_cell_path.len() + 1
    );

    let translated_level = SharingLevel::from_raw(child_facet_slot_sharing.raw().saturating_sub(1));
    shared_path_key(
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
    fn child_facet_slot_ancestor_translates_level0_to_full_path() {
        let path = vec![s("Div"), s("Dept"), s("Team")];
        let key = child_facet_slot_ancestor_key(&path, SharingLevel::from_raw(0), 4);
        assert_eq!(key, path);
    }

    #[test]
    fn child_facet_slot_ancestor_translates_level1_to_full_path() {
        let path = vec![s("Div"), s("Dept"), s("Team")];
        let key = child_facet_slot_ancestor_key(&path, SharingLevel::from_raw(1), 4);
        assert_eq!(key, path);
    }

    #[test]
    fn child_facet_slot_ancestor_translates_level2_to_drop_one() {
        let path = vec![s("Div"), s("Dept"), s("Team")];
        let key = child_facet_slot_ancestor_key(&path, SharingLevel::from_raw(2), 4);
        assert_eq!(key, vec![s("Div"), s("Dept")]);
    }

    #[test]
    fn child_facet_slot_ancestor_shared_like_levels_map_to_global() {
        let path = vec![s("Div"), s("Dept"), s("Team")];
        assert!(child_facet_slot_ancestor_key(&path, SharingLevel::from_raw(4), 4).is_empty());
        assert!(child_facet_slot_ancestor_key(&path, SharingLevel::from_raw(255), 4).is_empty());
    }
}
