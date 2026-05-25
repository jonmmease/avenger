use avenger_chart_core::{AxisOwnershipMode, FacetEmptyCellPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FacetOwnershipPolicy {
    pub(crate) effective_empty_cell_policy: FacetEmptyCellPolicy,
    pub(crate) has_holes: bool,
    pub(crate) axis_owner_ignore_empty_cells: bool,
    pub(crate) axis_ownership_mode: AxisOwnershipMode,
}

pub(crate) fn resolve_facet_ownership_policy(
    empty_cell_policy: FacetEmptyCellPolicy,
    has_holes: bool,
) -> FacetOwnershipPolicy {
    let axis_owner_ignore_empty_cells = empty_cell_policy.axis_owner_ignore_empty_cells(has_holes);
    FacetOwnershipPolicy {
        effective_empty_cell_policy: empty_cell_policy.effective(),
        has_holes,
        axis_owner_ignore_empty_cells,
        axis_ownership_mode: axis_ownership_mode_from_ignore_empty_cells(
            axis_owner_ignore_empty_cells,
        ),
    }
}

pub(crate) fn axis_ownership_mode_from_ignore_empty_cells(ignore: bool) -> AxisOwnershipMode {
    AxisOwnershipMode::from_ignore_empty_cells(ignore)
}

pub(crate) fn cell_requires_invalid_path_axis_fallback_hidden(
    is_empty_cell: bool,
    in_domain_slot: bool,
) -> bool {
    is_empty_cell && !in_domain_slot
}

pub(crate) fn has_holes_from_cells<I>(cell_is_empty: I) -> bool
where
    I: IntoIterator<Item = bool>,
{
    cell_is_empty.into_iter().any(|is_empty| is_empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_policy_hole_emptysubplot_auto_parity() {
        let hole = resolve_facet_ownership_policy(FacetEmptyCellPolicy::Hole, false);
        assert_eq!(hole.effective_empty_cell_policy, FacetEmptyCellPolicy::Hole);
        assert!(hole.axis_owner_ignore_empty_cells);
        assert_eq!(hole.axis_ownership_mode, AxisOwnershipMode::NonEmptySlots);

        let empty_subplot =
            resolve_facet_ownership_policy(FacetEmptyCellPolicy::EmptySubplot, true);
        assert_eq!(
            empty_subplot.effective_empty_cell_policy,
            FacetEmptyCellPolicy::EmptySubplot
        );
        assert!(!empty_subplot.axis_owner_ignore_empty_cells);
        assert_eq!(
            empty_subplot.axis_ownership_mode,
            AxisOwnershipMode::DomainSlots
        );

        let auto_without_holes = resolve_facet_ownership_policy(FacetEmptyCellPolicy::Auto, false);
        assert_eq!(
            auto_without_holes.effective_empty_cell_policy,
            FacetEmptyCellPolicy::Hole
        );
        assert!(!auto_without_holes.axis_owner_ignore_empty_cells);
        assert_eq!(
            auto_without_holes.axis_ownership_mode,
            AxisOwnershipMode::DomainSlots
        );

        let auto_with_holes = resolve_facet_ownership_policy(FacetEmptyCellPolicy::Auto, true);
        assert_eq!(
            auto_with_holes.effective_empty_cell_policy,
            FacetEmptyCellPolicy::Hole
        );
        assert!(auto_with_holes.axis_owner_ignore_empty_cells);
        assert_eq!(
            auto_with_holes.axis_ownership_mode,
            AxisOwnershipMode::NonEmptySlots
        );
    }

    #[test]
    fn axis_ownership_mode_mapping_matches_existing_behavior() {
        assert_eq!(
            axis_ownership_mode_from_ignore_empty_cells(false),
            AxisOwnershipMode::DomainSlots
        );
        assert_eq!(
            axis_ownership_mode_from_ignore_empty_cells(true),
            AxisOwnershipMode::NonEmptySlots
        );
    }

    #[test]
    fn invalid_path_axis_fallback_hidden_rule_matches_existing_behavior() {
        assert!(!cell_requires_invalid_path_axis_fallback_hidden(
            false, false
        ));
        assert!(!cell_requires_invalid_path_axis_fallback_hidden(
            false, true
        ));
        assert!(!cell_requires_invalid_path_axis_fallback_hidden(true, true));
        assert!(cell_requires_invalid_path_axis_fallback_hidden(true, false));
    }

    #[test]
    fn has_holes_from_cells_detects_any_empty_slot() {
        assert!(!has_holes_from_cells([false, false, false]));
        assert!(has_holes_from_cells([false, true, false]));
    }
}
