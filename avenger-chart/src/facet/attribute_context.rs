use datafusion::common::ScalarValue;
use ordered_float::OrderedFloat;

/// Scale selection scope for synthesized cell measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ScaleScopeKey {
    Shared,
    PerCellCached,
    PerCellFallback,
    AncestorCached,
    ExplicitBuilder,
}

/// Canonical inherited context key for synthesized cell measurements.
///
/// This is intentionally exact and stable across traversals within one evaluation run.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FacetInheritedContextKey {
    pub(crate) node_path: Vec<ScalarValue>,
    pub(crate) full_path: Vec<ScalarValue>,
    pub(crate) subplot_plot_width: OrderedFloat<f32>,
    pub(crate) subplot_plot_height: OrderedFloat<f32>,
    pub(crate) axis_owner_ignore_empty_cells: bool,
    pub(crate) invalid_path_axis_fallback_hidden: bool,
    pub(crate) scale_scope: ScaleScopeKey,
    pub(crate) coordinated_extents_fingerprint: u64,
}

impl FacetInheritedContextKey {
    pub(crate) fn from_parts(
        full_path: Vec<ScalarValue>,
        subplot_plot_width: f32,
        subplot_plot_height: f32,
        axis_owner_ignore_empty_cells: bool,
        invalid_path_axis_fallback_hidden: bool,
        scale_scope: ScaleScopeKey,
        coordinated_extents_fingerprint: u64,
    ) -> Self {
        let node_path = if full_path.is_empty() {
            Vec::new()
        } else {
            full_path[..full_path.len() - 1].to_vec()
        };
        Self {
            node_path,
            full_path,
            subplot_plot_width: OrderedFloat(subplot_plot_width),
            subplot_plot_height: OrderedFloat(subplot_plot_height),
            axis_owner_ignore_empty_cells,
            invalid_path_axis_fallback_hidden,
            scale_scope,
            coordinated_extents_fingerprint,
        }
    }
}

/// Back-compat alias for existing coordination/measurement code paths.
pub(crate) type FacetCellMeasureContextKey = FacetInheritedContextKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FacetCircularEpoch {
    CollectionA,
    InheritedApply,
    Recollection,
    InheritedPropagation,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct FacetInheritedContext {
    pub(crate) key: FacetInheritedContextKey,
    pub(crate) epoch: Option<FacetCircularEpoch>,
}
