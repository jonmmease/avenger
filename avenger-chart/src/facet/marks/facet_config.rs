use crate::channel::config_traits::ScaleSharing;
use crate::facet::empty_cell_policy::FacetEmptyCellPolicy;

#[derive(Clone, Default)]
pub struct FacetRowChannelConfig {
    pub(crate) title: Option<String>,
    pub(crate) slot_sharing: Option<ScaleSharing>,
    pub(crate) position: Option<String>,
    pub(crate) empty_cell_policy: Option<FacetEmptyCellPolicy>,
}

#[derive(Clone, Default)]
pub struct FacetOptions {
    pub(crate) title: Option<String>,
    pub(crate) slot_sharing: Option<ScaleSharing>,
    pub(crate) position: Option<String>,
    pub(crate) empty_cell_policy: Option<FacetEmptyCellPolicy>,
}

impl FacetOptions {
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Configure slot sharing mode for this facet variable.
    ///
    /// For nested facets (e.g., FacetRow inside FacetColumn):
    /// - `Shared`: All outer cells use the same slot set computed from the full dataset.
    ///   This creates a grid-like structure where empty cells may appear.
    /// - `Free` (default): Each outer cell computes its own slot set from filtered data.
    ///   Different outer cells may have different numbers of inner cells.
    ///
    /// Note: Free and Shared are normalized to Level(0) and Level(255) internally.
    pub fn with_slot_sharing(mut self, mode: ScaleSharing) -> Self {
        // Normalize Free/Shared to Level representation for internal consistency
        self.slot_sharing = Some(mode.to_normalized());
        self
    }

    /// Share this facet variable's slots across all facets (enumerate from the full dataset).
    pub fn share_slots(self) -> Self {
        self.with_slot_sharing(ScaleSharing::Shared)
    }

    /// Make this facet variable's slots independent per facet (enumerate from filtered data).
    pub fn free_slots(self) -> Self {
        self.with_slot_sharing(ScaleSharing::Free)
    }

    /// Set the position of the facet labels.
    ///
    /// - `"top"` (default): Labels appear above the plot area
    /// - `"bottom"`: Labels appear below the plot area
    pub fn position(mut self, position: impl Into<String>) -> Self {
        self.position = Some(position.into());
        self
    }

    /// Configure how empty facet cells are rendered.
    pub fn empty_cell_policy(mut self, policy: FacetEmptyCellPolicy) -> Self {
        self.empty_cell_policy = Some(policy);
        self
    }

    /// Render empty facet cells as holes.
    pub fn empty_cells_as_holes(self) -> Self {
        self.empty_cell_policy(FacetEmptyCellPolicy::Hole)
    }

    /// Render empty facet cells as empty subplots.
    pub fn empty_cells_as_subplots(self) -> Self {
        self.empty_cell_policy(FacetEmptyCellPolicy::EmptySubplot)
    }

    /// Resolve empty facet cells automatically (currently maps to holes).
    pub fn empty_cells_auto(self) -> Self {
        self.empty_cell_policy(FacetEmptyCellPolicy::Auto)
    }
}

impl FacetRowChannelConfig {
    pub fn facet<F>(mut self, f: F) -> Self
    where
        F: FnOnce(FacetOptions) -> FacetOptions,
    {
        let opts = f(FacetOptions::default());
        self.title = opts.title;
        self.slot_sharing = opts.slot_sharing;
        self.position = opts.position;
        self.empty_cell_policy = opts.empty_cell_policy;
        self
    }
}

#[derive(Clone, Default)]
pub struct FacetColChannelConfig {
    pub(crate) title: Option<String>,
    pub(crate) slot_sharing: Option<ScaleSharing>,
    pub(crate) position: Option<String>,
    pub(crate) empty_cell_policy: Option<FacetEmptyCellPolicy>,
}

impl FacetColChannelConfig {
    pub fn facet<F>(mut self, f: F) -> Self
    where
        F: FnOnce(FacetOptions) -> FacetOptions,
    {
        let opts = f(FacetOptions::default());
        self.title = opts.title;
        self.slot_sharing = opts.slot_sharing;
        self.position = opts.position;
        self.empty_cell_policy = opts.empty_cell_policy;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cells_as_subplots_sets_policy() {
        let options = FacetOptions::default().empty_cells_as_subplots();
        assert_eq!(
            options.empty_cell_policy,
            Some(FacetEmptyCellPolicy::EmptySubplot)
        );
    }

    #[test]
    fn empty_cells_auto_sets_auto_policy() {
        let options = FacetOptions::default().empty_cells_auto();
        assert_eq!(options.empty_cell_policy, Some(FacetEmptyCellPolicy::Auto));
    }
}
