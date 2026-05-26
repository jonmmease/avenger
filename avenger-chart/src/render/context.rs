//! Rendering context that carries theme and dimensions through the rendering pipeline
//!
//! The context is split into three parts:
//! - `EvaluationContext` - Constant across entire evaluate() call, built once at top level
//! - `RenderState` - Changes per subplot, contains computed dimensions and scales
//! - `RenderContext` - Thin facade combining both for mark rendering API

use std::{
    collections::HashMap,
    ops::Deref,
    sync::{Arc, Mutex},
};

use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;

use avenger_chart_core::{
    EvaluationContext as CoreEvaluationContext, MarkRenderContext as CoreMarkRenderContext,
    MarkRuntimeContext,
};

use crate::{
    container::{ChildFrameSharingLevel, ChildFrameSharingPath, ContainerPathSegment},
    coords::CoordMeasurement,
    coords::FacetAxis,
    facet::{
        evaluated_facet_tree::EvaluatedFacetTree,
        layout_plan::{FacetBandPaddingFeedback, FacetBandPaddingFeedbackMap},
        scale_precompute::FacetScalePrecomputeStore,
    },
    render::types::{
        EvaluatedPlot, EvaluationMetrics, FacetLayoutRefinement, FacetSubtreeSnapshot,
        LayoutDebugOverlayMode,
    },
    scales::ConfiguredScaleWithSpec,
    theme::{Theme, ThemeContext, ThemeValue},
};

pub use avenger_chart_core::{
    AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM, INVALID_FACET_PATH_AXIS_FALLBACK_HIDDEN_PARAM,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PhysicalDimension {
    Width,
    Height,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FacetDimensionSizing {
    CanvasConstrained { canvas_size: f32 },
    LeafPlotAreaSized { leaf_plot_size: f32 },
}

impl FacetDimensionSizing {
    pub(crate) fn is_canvas_constrained(self) -> bool {
        matches!(self, Self::CanvasConstrained { .. })
    }

    pub(crate) fn is_leaf_plot_area_sized(self) -> bool {
        matches!(self, Self::LeafPlotAreaSized { .. })
    }

    pub(crate) fn leaf_plot_size(self) -> Option<f32> {
        match self {
            Self::LeafPlotAreaSized { leaf_plot_size } => Some(leaf_plot_size),
            Self::CanvasConstrained { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FacetRuntimeSizingPolicy {
    pub(crate) width: FacetDimensionSizing,
    pub(crate) height: FacetDimensionSizing,
}

impl FacetRuntimeSizingPolicy {
    pub(crate) fn fully_canvas_constrained(canvas_width: f32, canvas_height: f32) -> Self {
        Self {
            width: FacetDimensionSizing::CanvasConstrained {
                canvas_size: canvas_width,
            },
            height: FacetDimensionSizing::CanvasConstrained {
                canvas_size: canvas_height,
            },
        }
    }

    pub(crate) fn fully_leaf_plot_area_sized(leaf_plot_width: f32, leaf_plot_height: f32) -> Self {
        Self {
            width: FacetDimensionSizing::LeafPlotAreaSized {
                leaf_plot_size: leaf_plot_width,
            },
            height: FacetDimensionSizing::LeafPlotAreaSized {
                leaf_plot_size: leaf_plot_height,
            },
        }
    }

    pub(crate) fn dimension(self, dimension: PhysicalDimension) -> FacetDimensionSizing {
        match dimension {
            PhysicalDimension::Width => self.width,
            PhysicalDimension::Height => self.height,
        }
    }

    pub(crate) fn facet_band_dimension(self, axis: FacetAxis) -> FacetDimensionSizing {
        self.dimension(facet_band_physical_dimension(axis))
    }

    pub(crate) fn facet_orthogonal_dimension(self, axis: FacetAxis) -> FacetDimensionSizing {
        self.dimension(facet_orthogonal_physical_dimension(axis))
    }

    pub(crate) fn is_fully_canvas_constrained(self) -> bool {
        self.width.is_canvas_constrained() && self.height.is_canvas_constrained()
    }

    pub(crate) fn is_fully_leaf_plot_area_sized(self) -> bool {
        self.width.is_leaf_plot_area_sized() && self.height.is_leaf_plot_area_sized()
    }

    pub(crate) fn has_leaf_plot_area_sized_dimension(self) -> bool {
        self.width.is_leaf_plot_area_sized() || self.height.is_leaf_plot_area_sized()
    }

    pub(crate) fn leaf_plot_width(self) -> Option<f32> {
        self.width.leaf_plot_size()
    }

    pub(crate) fn leaf_plot_height(self) -> Option<f32> {
        self.height.leaf_plot_size()
    }
}

pub(crate) fn facet_band_physical_dimension(axis: FacetAxis) -> PhysicalDimension {
    match axis {
        FacetAxis::Column => PhysicalDimension::Width,
        FacetAxis::Row => PhysicalDimension::Height,
    }
}

pub(crate) fn facet_orthogonal_physical_dimension(axis: FacetAxis) -> PhysicalDimension {
    match axis {
        FacetAxis::Column => PhysicalDimension::Height,
        FacetAxis::Row => PhysicalDimension::Width,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FacetRuntimeSizingMode {
    CanvasFit,
    Policy(FacetRuntimeSizingPolicy),
}

impl FacetRuntimeSizingMode {
    pub(crate) fn policy(self) -> FacetRuntimeSizingPolicy {
        match self {
            Self::CanvasFit => FacetRuntimeSizingPolicy::fully_canvas_constrained(0.0, 0.0),
            Self::Policy(policy) => policy,
        }
    }

    pub(crate) fn has_leaf_plot_area_sized_dimension(self) -> bool {
        self.policy().has_leaf_plot_area_sized_dimension()
    }

    pub(crate) fn facet_band_is_leaf_plot_area_sized(self, axis: FacetAxis) -> bool {
        self.policy()
            .facet_band_dimension(axis)
            .is_leaf_plot_area_sized()
    }
}

pub(crate) struct FacetSubtreeSnapshotCapture {
    pub(crate) request: FacetSubtreeSnapshot,
    pub(crate) result: Option<EvaluatedPlot>,
}

/// Immutable context built once at evaluate() entry.
///
/// Contains all state that remains constant throughout the entire evaluation:
/// - Theme for styling
/// - DataFusion session for data operations
/// - Runtime parameters
/// - Pre-computed facet structure
#[derive(Clone)]
pub struct EvaluationContext {
    /// Stable evaluation inputs that belong at the future core crate boundary.
    pub(crate) core: CoreEvaluationContext,
    /// Pre-computed facet structure for efficient domain lookups and visibility decisions.
    pub facet_tree: Arc<EvaluatedFacetTree>,
    /// Unfiltered data inherited by the current facet tree, used by mark-level
    /// facet data scopes that look above the current cell.
    pub(crate) facet_data_root: Option<DataFrame>,
    /// Whether invalid facet paths should hide axis labels/titles instead of showing them.
    ///
    /// This is used when rendering placeholder facet slots as empty subplots to avoid
    /// duplicate ownership labels on non-owner paths.
    pub hide_invalid_facet_path_axes: bool,
    /// Shared cache of facet scale precompute artifacts for the current evaluation run.
    pub(crate) facet_scale_precompute_store: Arc<FacetScalePrecomputeStore>,
    /// Internal facet runtime sizing policy used while measuring and coordinating facets.
    pub(crate) facet_runtime_sizing_mode: FacetRuntimeSizingMode,
    /// Effective layout debug overlay mode.
    pub(crate) debug_layout_overlay: LayoutDebugOverlayMode,
    /// Facet refinement policy for final layout evaluation.
    pub(crate) facet_layout_refinement: FacetLayoutRefinement,
    /// Optional estimated-overflow probe-size seed from a previous realized facet tree.
    pub(crate) facet_probe_size_overrides: Option<Arc<HashMap<Vec<ScalarValue>, (f32, f32)>>>,
    /// Optional realized inner-padding lower bounds from the previous refinement pass.
    pub(crate) facet_padding_feedback: Option<Arc<FacetBandPaddingFeedbackMap>>,
    /// Child-index path from the root facet coord to the currently evaluated facet cell.
    pub(crate) facet_coord_node_path: Vec<usize>,
    /// Generic ancestor child-frame path for the currently evaluated child plot.
    ///
    /// Facet, concat, and future child-frame containers extend this as they
    /// measure nested child plots. Coordination scope keys use it to distinguish
    /// semantically equivalent nested containers that live under different
    /// parent children.
    pub(crate) child_frame_container_path: Vec<ContainerPathSegment>,
    /// Generic nested child-frame sharing path for the currently evaluated child plot.
    ///
    /// This records child index/count metadata for container levels such as
    /// concat, allowing guides and legends to apply sharing ownership without
    /// depending on a facet path.
    pub(crate) child_frame_sharing_path: ChildFrameSharingPath,
    /// Optional shared collector for focused evaluation diagnostics.
    pub(crate) evaluation_metrics: Option<Arc<Mutex<EvaluationMetrics>>>,
    /// Optional one-shot facet-subtree snapshot capture for non-renderable intermediate states.
    pub(crate) facet_subtree_snapshot_capture: Option<Arc<Mutex<FacetSubtreeSnapshotCapture>>>,
}

impl EvaluationContext {
    pub fn new(
        theme: Arc<Theme>,
        session_context: Arc<SessionContext>,
        params: IndexMap<String, ScalarValue>,
        facet_tree: Arc<EvaluatedFacetTree>,
    ) -> Self {
        Self {
            core: CoreEvaluationContext::new(theme, session_context, params),
            facet_tree,
            facet_data_root: None,
            hide_invalid_facet_path_axes: false,
            facet_scale_precompute_store: Arc::new(FacetScalePrecomputeStore::default()),
            facet_runtime_sizing_mode: FacetRuntimeSizingMode::CanvasFit,
            debug_layout_overlay: LayoutDebugOverlayMode::Off,
            facet_layout_refinement: FacetLayoutRefinement::default(),
            facet_probe_size_overrides: None,
            facet_padding_feedback: None,
            facet_coord_node_path: Vec::new(),
            child_frame_container_path: Vec::new(),
            child_frame_sharing_path: ChildFrameSharingPath::root(),
            evaluation_metrics: None,
            facet_subtree_snapshot_capture: None,
        }
    }

    /// Create a new context with different params, reusing other fields (cheap Arc clones)
    pub fn with_params(&self, params: IndexMap<String, ScalarValue>) -> Self {
        Self {
            core: self.core.with_params(params),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    /// Create a new context with canvas dimensions added to params (for media queries)
    pub fn with_dimension_params(&self, width: f32, height: f32) -> Self {
        Self {
            core: self.core.with_dimension_params(width, height),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    /// Create a new context overriding invalid facet-path axis fallback behavior.
    pub fn with_invalid_facet_path_axis_fallback_hidden(&self, hidden: bool) -> Self {
        let mut params = self.params.clone();
        params.insert(
            INVALID_FACET_PATH_AXIS_FALLBACK_HIDDEN_PARAM.to_string(),
            ScalarValue::Boolean(Some(hidden)),
        );
        Self {
            core: self.core.with_params(params),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: hidden,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    /// Create a new context overriding whether axis ownership should ignore empty cells.
    pub fn with_axis_owner_ignore_empty_cells(&self, ignore_empty_cells: bool) -> Self {
        let mut params = self.params.clone();
        params.insert(
            AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM.to_string(),
            ScalarValue::Boolean(Some(ignore_empty_cells)),
        );
        Self {
            core: self.core.with_params(params),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn with_facet_runtime_sizing_mode(&self, mode: FacetRuntimeSizingMode) -> Self {
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn facet_runtime_sizing_mode(&self) -> FacetRuntimeSizingMode {
        self.facet_runtime_sizing_mode
    }

    pub(crate) fn with_facet_data_root(&self, facet_data_root: Option<DataFrame>) -> Self {
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root,
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn facet_data_root(&self) -> Option<&DataFrame> {
        self.facet_data_root.as_ref()
    }

    pub(crate) fn with_debug_layout_overlay(&self, mode: LayoutDebugOverlayMode) -> Self {
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: mode,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn with_facet_layout_refinement(&self, refinement: FacetLayoutRefinement) -> Self {
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn facet_layout_refinement(&self) -> FacetLayoutRefinement {
        self.facet_layout_refinement
    }

    pub(crate) fn with_facet_probe_size_overrides(
        &self,
        overrides: Arc<HashMap<Vec<ScalarValue>, (f32, f32)>>,
    ) -> Self {
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: Some(overrides),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn facet_probe_size_override(
        &self,
        facet_path: &[ScalarValue],
    ) -> Option<(f32, f32)> {
        self.facet_probe_size_overrides
            .as_ref()
            .and_then(|overrides| overrides.get(facet_path).copied())
    }

    pub(crate) fn with_facet_padding_feedback(
        &self,
        feedback: Arc<FacetBandPaddingFeedbackMap>,
    ) -> Self {
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: Some(feedback),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn facet_padding_feedback(
        &self,
        facet_coord_node_path: &[usize],
    ) -> Option<FacetBandPaddingFeedback> {
        self.facet_padding_feedback
            .as_ref()
            .and_then(|feedback| feedback.get(facet_coord_node_path).copied())
    }

    pub(crate) fn debug_layout_overlay(&self) -> LayoutDebugOverlayMode {
        self.debug_layout_overlay
    }

    pub(crate) fn facet_scale_precompute_store(&self) -> &Arc<FacetScalePrecomputeStore> {
        &self.facet_scale_precompute_store
    }

    pub(crate) fn with_evaluation_metrics(&self, metrics: Arc<Mutex<EvaluationMetrics>>) -> Self {
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: Some(metrics),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn with_facet_subtree_snapshot_capture(
        &self,
        capture: Arc<Mutex<FacetSubtreeSnapshotCapture>>,
    ) -> Self {
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: Some(capture),
        }
    }

    pub(crate) fn with_facet_coord_node_path_appended(&self, child_index: usize) -> Self {
        let mut facet_coord_node_path = self.facet_coord_node_path.clone();
        facet_coord_node_path.push(child_index);
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path,
            child_frame_container_path: self.child_frame_container_path.clone(),
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn facet_coord_node_path(&self) -> &[usize] {
        &self.facet_coord_node_path
    }

    pub(crate) fn child_frame_container_path(&self) -> &[ContainerPathSegment] {
        &self.child_frame_container_path
    }

    pub(crate) fn child_frame_sharing_path(&self) -> &ChildFrameSharingPath {
        &self.child_frame_sharing_path
    }

    pub(crate) fn with_child_frame_container_path_appended(
        &self,
        segment: ContainerPathSegment,
    ) -> Self {
        let mut child_frame_container_path = self.child_frame_container_path.clone();
        child_frame_container_path.push(segment);
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path,
            child_frame_sharing_path: self.child_frame_sharing_path.clone(),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn with_child_frame_sharing_level_appended(
        &self,
        level: ChildFrameSharingLevel,
    ) -> Self {
        let mut child_frame_container_path = self.child_frame_container_path.clone();
        child_frame_container_path.push(level.segment.clone());
        Self {
            core: self.core.clone(),
            facet_tree: self.facet_tree.clone(),
            facet_data_root: self.facet_data_root.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_overlay: self.debug_layout_overlay,
            facet_layout_refinement: self.facet_layout_refinement,
            facet_probe_size_overrides: self.facet_probe_size_overrides.clone(),
            facet_padding_feedback: self.facet_padding_feedback.clone(),
            facet_coord_node_path: self.facet_coord_node_path.clone(),
            child_frame_container_path,
            child_frame_sharing_path: self.child_frame_sharing_path.appended(level),
            evaluation_metrics: self.evaluation_metrics.clone(),
            facet_subtree_snapshot_capture: self.facet_subtree_snapshot_capture.clone(),
        }
    }

    pub(crate) fn facet_subtree_snapshot_request(&self) -> Option<FacetSubtreeSnapshot> {
        self.facet_subtree_snapshot_capture.as_ref().map(|capture| {
            capture
                .lock()
                .expect("facet subtree snapshot capture lock poisoned")
                .request
                .clone()
        })
    }

    pub(crate) fn capture_facet_subtree_snapshot(
        &self,
        request: &FacetSubtreeSnapshot,
        evaluated: EvaluatedPlot,
    ) -> bool {
        let Some(capture) = &self.facet_subtree_snapshot_capture else {
            return false;
        };
        let mut capture = capture
            .lock()
            .expect("facet subtree snapshot capture lock poisoned");
        if capture.result.is_none() && capture.request == *request {
            capture.result = Some(evaluated);
            true
        } else {
            false
        }
    }

    pub(crate) fn take_facet_subtree_snapshot(&self) -> Option<EvaluatedPlot> {
        self.facet_subtree_snapshot_capture
            .as_ref()
            .and_then(|capture| {
                capture
                    .lock()
                    .expect("facet subtree snapshot capture lock poisoned")
                    .result
                    .take()
            })
    }

    pub(crate) fn record_plot_component_measure_call(&self, facet_depth: usize) {
        if let Some(metrics) = &self.evaluation_metrics {
            metrics
                .lock()
                .expect("evaluation metrics lock poisoned")
                .record_plot_component_measure_call(facet_depth);
        }
    }

    pub(crate) fn record_facet_band_measure_run(
        &self,
        estimated_overflow_leaf_measure_count: usize,
        estimated_overflow_non_leaf_aggregate_count: usize,
        estimated_overflow_non_leaf_full_measure_count: usize,
    ) {
        if let Some(metrics) = &self.evaluation_metrics {
            metrics
                .lock()
                .expect("evaluation metrics lock poisoned")
                .record_facet_band_measure_run(
                    estimated_overflow_leaf_measure_count,
                    estimated_overflow_non_leaf_aggregate_count,
                    estimated_overflow_non_leaf_full_measure_count,
                );
        }
    }

    pub(crate) fn record_facet_refinement_pass(&self) {
        if let Some(metrics) = &self.evaluation_metrics {
            metrics
                .lock()
                .expect("evaluation metrics lock poisoned")
                .record_facet_refinement_pass();
        }
    }

    pub(crate) fn record_facet_refinement_converged(&self) {
        if let Some(metrics) = &self.evaluation_metrics {
            metrics
                .lock()
                .expect("evaluation metrics lock poisoned")
                .record_facet_refinement_converged();
        }
    }

    pub(crate) fn record_facet_refinement_hit_max_passes(&self) {
        if let Some(metrics) = &self.evaluation_metrics {
            metrics
                .lock()
                .expect("evaluation metrics lock poisoned")
                .record_facet_refinement_hit_max_passes();
        }
    }
}

impl Deref for EvaluationContext {
    type Target = CoreEvaluationContext;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

/// State that changes per subplot during rendering traversal.
///
/// Created fresh for each subplot with its computed dimensions and scales.
#[derive(Clone)]
pub struct RenderState {
    /// Width of the plot area
    pub plot_width: f32,
    /// Height of the plot area
    pub plot_height: f32,
    /// Configured scales available during rendering (coordinate + non-positional)
    pub scales: HashMap<String, ConfiguredScaleWithSpec>,
}

impl RenderState {
    pub fn new(
        plot_width: f32,
        plot_height: f32,
        scales: HashMap<String, ConfiguredScaleWithSpec>,
    ) -> Self {
        Self {
            plot_width,
            plot_height,
            scales,
        }
    }
}

/// Combined view for mark rendering.
///
/// This is a facade that combines references to `EvaluationContext` and `RenderState`,
/// plus the current facet path. It provides the full rendering context needed by marks.
pub struct RenderContext<'a> {
    /// Reference to the evaluation-level context (constant across evaluate())
    pub eval: &'a EvaluationContext,
    /// Reference to the subplot-level state (varies per subplot)
    pub state: &'a RenderState,
    /// Current cell path in facet hierarchy (values at each nesting level).
    /// Empty when not inside a facet cell. e.g., `["East", "Eng"]`
    pub facet_path: &'a [ScalarValue],
    /// Coordinate-system-specific measurement data (e.g., facet cell layout).
    /// For facet coordinate systems this contains subplot measurements.
    /// For non-facet coordinate systems this is `EmptyCoordMeasurement`.
    pub coord_measurement: &'a dyn CoordMeasurement,
}

impl<'a> RenderContext<'a> {
    /// Create a RenderContext with coordinate measurement
    pub fn new(
        eval: &'a EvaluationContext,
        state: &'a RenderState,
        facet_path: &'a [ScalarValue],
        coord_measurement: &'a dyn CoordMeasurement,
    ) -> Self {
        Self {
            eval,
            state,
            facet_path,
            coord_measurement,
        }
    }

    /// Get coordinate measurement
    pub fn coord_measurement(&self) -> &dyn CoordMeasurement {
        self.coord_measurement
    }

    /// Return the stable core render view for mark implementations.
    pub fn core_view(&self) -> CoreMarkRenderContext<'a> {
        CoreMarkRenderContext::new(self.eval, self.plot_width(), self.plot_height())
    }

    // Convenience accessors that delegate to inner structs

    /// Get the theme
    pub fn theme(&self) -> &Arc<Theme> {
        self.eval.theme()
    }

    /// Get the session context
    pub fn session_context(&self) -> &Arc<SessionContext> {
        self.eval.session_context()
    }

    /// Get the params
    pub fn params(&self) -> &IndexMap<String, ScalarValue> {
        self.eval.params()
    }

    /// Get the facet tree
    pub fn facet_tree(&self) -> &EvaluatedFacetTree {
        &self.eval.facet_tree
    }

    /// Get plot width
    pub fn plot_width(&self) -> f32 {
        self.state.plot_width
    }

    /// Get plot height
    pub fn plot_height(&self) -> f32 {
        self.state.plot_height
    }

    /// Get scales
    pub fn scales(&self) -> &HashMap<String, ConfiguredScaleWithSpec> {
        &self.state.scales
    }

    /// Query theme property with automatic parameter resolution
    ///
    /// This is a convenience method that combines theme querying with parameter resolution.
    /// It resolves:
    /// - CSS variables (var()) using params or theme defaults
    /// - light-dark() functions using the "color-scheme" param
    pub fn query_theme(&self, context: &ThemeContext, property: &str) -> Option<ThemeValue> {
        self.eval.query_theme(context, property)
    }

    /// Get font size with parameter support
    pub fn font_size(&self, context: &ThemeContext) -> Option<f32> {
        self.eval.font_size(context)
    }
}

impl MarkRuntimeContext for RenderContext<'_> {
    fn core_view(&self) -> CoreMarkRenderContext<'_> {
        CoreMarkRenderContext::new(self.eval, self.state.plot_width, self.state.plot_height)
    }

    fn coord_measurement(&self) -> &dyn CoordMeasurement {
        self.coord_measurement
    }

    fn facet_path(&self) -> &[ScalarValue] {
        self.facet_path
    }
}
