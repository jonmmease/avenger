use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{AxisPosition, CoordinationAxis, SharingLevel};

/// Policy for deciding which repeated/faceted axes should show labels and titles.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisGuideVisibilityPolicy {
    /// Preserve the container's current/default guide compaction behavior.
    #[default]
    Auto,
    /// Show labels/titles on every axis.
    All,
    /// Show labels/titles only on physical outer/non-empty edge axes.
    OuterEdges,
    /// Compact only when aligned axes are semantically equivalent.
    ///
    /// This variant is reserved for repeat/matrix-style semantics. Containers
    /// that do not have enough equivalence metadata should fall back to their
    /// safe default behavior.
    OuterForEquivalentDomainGroups,
}

/// Separate visibility policies for tick labels and axis titles.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisGuideVisibilityConfig {
    pub labels: AxisGuideVisibilityPolicy,
    pub title: AxisGuideVisibilityPolicy,
}

impl AxisGuideVisibilityConfig {
    pub fn new(labels: AxisGuideVisibilityPolicy, title: AxisGuideVisibilityPolicy) -> Self {
        Self { labels, title }
    }

    pub fn same(policy: AxisGuideVisibilityPolicy) -> Self {
        Self {
            labels: policy,
            title: policy,
        }
    }

    pub fn auto() -> Self {
        Self::same(AxisGuideVisibilityPolicy::Auto)
    }
}

#[doc(hidden)]
pub const INVALID_FACET_PATH_AXIS_FALLBACK_HIDDEN_PARAM: &str =
    "__avenger_hide_invalid_facet_path_axes";
#[doc(hidden)]
pub const AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM: &str = "__avenger_axis_owner_ignore_empty_cells";

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

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisOwnershipMode {
    /// Compute owners against the full slot geometry.
    DomainSlots,
    /// Compute owners against non-empty cells only (hole-aware behavior).
    NonEmptySlots,
}

impl AxisOwnershipMode {
    #[doc(hidden)]
    pub fn from_ignore_empty_cells(ignore_empty_cells: bool) -> Self {
        if ignore_empty_cells {
            Self::NonEmptySlots
        } else {
            Self::DomainSlots
        }
    }
}

#[doc(hidden)]
pub fn axis_owner_ignore_empty_cells_from_params(params: &IndexMap<String, ScalarValue>) -> bool {
    params
        .get(AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM)
        .and_then(|value| match value {
            ScalarValue::Boolean(Some(value)) => Some(*value),
            _ => None,
        })
        .unwrap_or(false)
}

#[doc(hidden)]
pub fn axis_ownership_mode_from_params(
    params: &IndexMap<String, ScalarValue>,
) -> AxisOwnershipMode {
    AxisOwnershipMode::from_ignore_empty_cells(axis_owner_ignore_empty_cells_from_params(params))
}

#[doc(hidden)]
pub trait FacetGuideSharingView: Send + Sync {
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

    fn axis_guide_visibility_config_for_path(
        &self,
        _path: &[ScalarValue],
        _axis_position: AxisPosition,
    ) -> Option<AxisGuideVisibilityConfig> {
        None
    }

    fn effective_edge_indices_for_values_at_path(
        &self,
        facet_path: &[ScalarValue],
        values: &[ScalarValue],
    ) -> Option<(usize, usize)>;
}

#[doc(hidden)]
pub trait ChildFrameGuideSharingView: Send + Sync {
    fn position_indices(&self) -> Vec<usize>;

    fn level_counts(&self) -> Vec<usize>;

    fn level_axes(&self) -> Vec<CoordinationAxis>;

    fn axis_guide_visibility_config_for_axis(
        &self,
        _axis: CoordinationAxis,
    ) -> AxisGuideVisibilityConfig {
        AxisGuideVisibilityConfig::auto()
    }

    fn relevant_depth(&self, axis: CoordinationAxis) -> usize {
        self.level_axes()
            .into_iter()
            .filter(|level_axis| *level_axis == axis)
            .count()
    }
}
