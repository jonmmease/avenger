use datafusion::common::ScalarValue;

use crate::{
    chart_core::AxisPosition,
    plot::compiled::{CoordinationAxis, SharingLevel},
};

/// Result of guide axis visibility computation for a container cell.
///
/// This is intentionally guide-facing rather than facet-specific. Facets are
/// one provider of these decisions, but coordinate guides only need the final
/// label/title ownership result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AxisVisibility {
    /// Whether to show tick labels on this axis.
    pub show_labels: bool,
    /// Whether to show the axis title.
    pub show_title: bool,
}

impl AxisVisibility {
    /// Create visibility with both labels and title shown.
    pub fn visible() -> Self {
        Self {
            show_labels: true,
            show_title: true,
        }
    }

    /// Create visibility with both labels and title hidden.
    pub fn hidden() -> Self {
        Self {
            show_labels: false,
            show_title: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AxisOwnershipMode {
    /// Compute owners against the full slot geometry.
    DomainSlots,
    /// Compute owners against non-empty cells only (hole-aware behavior).
    NonEmptySlots,
}

pub(crate) trait FacetGuideSharingView: Send + Sync {
    fn channel_axis_visibility_for_path_checked(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
        sharing_level: u8,
    ) -> Option<AxisVisibility>;

    fn channel_axis_visibility_for_path_checked_with_mode(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
        sharing_level: u8,
        ownership_mode: AxisOwnershipMode,
    ) -> Option<AxisVisibility>;

    fn is_jagged_for_axis(&self, axis_position: AxisPosition) -> bool;

    fn channel_domain_sharing_level(&self, channel: &str) -> SharingLevel;

    fn effective_edge_indices_for_values_at_path(
        &self,
        facet_path: &[ScalarValue],
        values: &[ScalarValue],
    ) -> Option<(usize, usize)>;
}

pub(crate) trait ChildFrameGuideSharingView: Send + Sync {
    fn position_indices(&self) -> Vec<usize>;

    fn level_counts(&self) -> Vec<usize>;

    fn level_axes(&self) -> Vec<CoordinationAxis>;

    fn relevant_depth(&self, axis: CoordinationAxis) -> usize {
        self.level_axes()
            .into_iter()
            .filter(|level_axis| *level_axis == axis)
            .count()
    }
}
