use serde::{Deserialize, Serialize};

/// Rendering policy for empty facet cells.
///
/// Empty cells include:
/// - slot placeholders introduced by shared facet slot sets
/// - cells whose slot exists but has zero rows after filtering
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacetEmptyCellPolicy {
    /// Reserve slot geometry but render no subplot contents.
    Hole,
    /// Render subplot scaffolding (axes/guides/grid) with no data marks.
    EmptySubplot,
    /// Deferred auto policy.
    ///
    /// In this release Auto resolves deterministically to [`FacetEmptyCellPolicy::Hole`].
    #[default]
    Auto,
}

impl FacetEmptyCellPolicy {
    /// Resolve to the effective concrete rendering policy for this release.
    pub fn effective(self) -> Self {
        match self {
            Self::Auto => Self::Hole,
            mode => mode,
        }
    }

    /// Determine whether cartesian axis owner resolution should ignore empty cells.
    ///
    /// This can differ from `effective()`:
    /// - `Hole` always ignores empty cells
    /// - `EmptySubplot` never ignores empty cells
    /// - `Auto` ignores empty cells only when the current facet level actually has holes
    pub fn axis_owner_ignore_empty_cells(self, has_holes: bool) -> bool {
        match self {
            Self::Hole => true,
            Self::EmptySubplot => false,
            Self::Auto => has_holes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FacetEmptyCellPolicy;

    #[test]
    fn default_is_auto() {
        assert_eq!(FacetEmptyCellPolicy::default(), FacetEmptyCellPolicy::Auto);
    }

    #[test]
    fn serde_round_trip_snake_case() {
        let encoded = serde_json::to_string(&FacetEmptyCellPolicy::EmptySubplot).unwrap();
        assert_eq!(encoded, "\"empty_subplot\"");

        let decoded: FacetEmptyCellPolicy = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, FacetEmptyCellPolicy::EmptySubplot);
    }

    #[test]
    fn auto_effective_is_hole() {
        assert_eq!(
            FacetEmptyCellPolicy::Auto.effective(),
            FacetEmptyCellPolicy::Hole
        );
    }

    #[test]
    fn axis_owner_ignore_empty_cells_behaves_by_policy() {
        assert!(FacetEmptyCellPolicy::Hole.axis_owner_ignore_empty_cells(false));
        assert!(!FacetEmptyCellPolicy::EmptySubplot.axis_owner_ignore_empty_cells(true));
        assert!(!FacetEmptyCellPolicy::Auto.axis_owner_ignore_empty_cells(false));
        assert!(FacetEmptyCellPolicy::Auto.axis_owner_ignore_empty_cells(true));
    }
}
