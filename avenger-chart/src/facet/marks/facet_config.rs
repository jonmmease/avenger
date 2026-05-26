use avenger_chart_core::{DefaultLogicalExprNodeExt, FacetEmptyCellPolicy, IntoExpr, ScaleSharing};
use datafusion_proto::protobuf::LogicalExprNode;

#[derive(Clone, Default)]
pub struct FacetRowChannelConfig {
    pub(crate) title: Option<String>,
    pub(crate) slot_sharing: Option<ScaleSharing>,
    pub(crate) position: Option<String>,
    pub(crate) visible: Option<bool>,
    pub(crate) empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) order_expr: Option<LogicalExprNode>,
    pub(crate) order_descending: bool,
}

#[derive(Clone, Default)]
pub struct FacetGuideOptions {
    pub(crate) title: Option<String>,
    pub(crate) position: Option<String>,
    pub(crate) visible: Option<bool>,
}

impl FacetGuideOptions {
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the position of the facet labels.
    ///
    /// - `"top"` (default): Labels appear above the plot area
    /// - `"bottom"`: Labels appear below the plot area
    pub fn position(mut self, position: impl Into<String>) -> Self {
        self.position = Some(position.into());
        self
    }

    /// Configure whether this facet guide is rendered.
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = Some(visible);
        self
    }
}

impl FacetRowChannelConfig {
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

    pub fn order_by(mut self, expr: impl IntoExpr) -> Self {
        self.order_expr = Some(
            LogicalExprNode::from_default_expr(expr.into_expr())
                .expect("Failed to serialize facet row order expression"),
        );
        self
    }

    pub fn order_asc(mut self) -> Self {
        self.order_descending = false;
        self
    }

    pub fn order_desc(mut self) -> Self {
        self.order_descending = true;
        self
    }

    pub fn guide<F>(mut self, f: F) -> Self
    where
        F: FnOnce(FacetGuideOptions) -> FacetGuideOptions,
    {
        let opts = f(FacetGuideOptions::default());
        self.title = opts.title;
        self.position = opts.position;
        self.visible = opts.visible;
        self
    }
}

#[derive(Clone, Default)]
pub struct FacetColChannelConfig {
    pub(crate) title: Option<String>,
    pub(crate) slot_sharing: Option<ScaleSharing>,
    pub(crate) position: Option<String>,
    pub(crate) visible: Option<bool>,
    pub(crate) empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) order_expr: Option<LogicalExprNode>,
    pub(crate) order_descending: bool,
}

impl FacetColChannelConfig {
    /// Configure slot sharing mode for this facet variable.
    ///
    /// For nested facets (e.g., FacetColumn inside FacetRow):
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

    pub fn order_by(mut self, expr: impl IntoExpr) -> Self {
        self.order_expr = Some(
            LogicalExprNode::from_default_expr(expr.into_expr())
                .expect("Failed to serialize facet column order expression"),
        );
        self
    }

    pub fn order_asc(mut self) -> Self {
        self.order_descending = false;
        self
    }

    pub fn order_desc(mut self) -> Self {
        self.order_descending = true;
        self
    }

    pub fn guide<F>(mut self, f: F) -> Self
    where
        F: FnOnce(FacetGuideOptions) -> FacetGuideOptions,
    {
        let opts = f(FacetGuideOptions::default());
        self.title = opts.title;
        self.position = opts.position;
        self.visible = opts.visible;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cells_as_subplots_sets_policy() {
        let config = FacetRowChannelConfig::default().empty_cells_as_subplots();
        assert_eq!(
            config.empty_cell_policy,
            Some(FacetEmptyCellPolicy::EmptySubplot)
        );
    }

    #[test]
    fn empty_cells_auto_sets_auto_policy() {
        let config = FacetColChannelConfig::default().empty_cells_auto();
        assert_eq!(config.empty_cell_policy, Some(FacetEmptyCellPolicy::Auto));
    }

    #[test]
    fn guide_visible_false_sets_visible_false() {
        let options = FacetGuideOptions::default().visible(false);
        assert_eq!(options.visible, Some(false));
    }
}
