//! Rendering context that carries theme and dimensions through the rendering pipeline
//!
//! The context is split into three parts:
//! - `EvaluationContext` - Constant across entire evaluate() call, built once at top level
//! - `RenderState` - Changes per subplot, contains computed dimensions and scales
//! - `RenderContext` - Thin facade combining both for mark rendering API

use std::{collections::HashMap, sync::Arc};

use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{
    coords::CoordMeasurement,
    facet::{
        evaluated_facet_tree::EvaluatedFacetTree, scale_precompute::FacetScalePrecomputeStore,
    },
    scales::ConfiguredScaleWithSpec,
    theme::{Theme, ThemeContext, ThemeValue},
};

pub const INVALID_FACET_PATH_AXIS_FALLBACK_HIDDEN_PARAM: &str =
    "__avenger_hide_invalid_facet_path_axes";
pub const AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM: &str = "__avenger_axis_owner_ignore_empty_cells";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FacetRuntimeSizingMode {
    CanvasFit,
    FixedSubplot {
        leaf_plot_width: f32,
        leaf_plot_height: f32,
    },
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
    /// The theme to use for rendering
    pub theme: Arc<Theme>,
    /// The DataFusion session context for DataFrame operations
    pub session_context: Arc<SessionContext>,
    /// Parameter values for prepared statements
    pub params: IndexMap<String, ScalarValue>,
    /// Pre-computed facet structure for efficient domain lookups and visibility decisions.
    pub facet_tree: Arc<EvaluatedFacetTree>,
    /// Whether invalid facet paths should hide axis labels/titles instead of showing them.
    ///
    /// This is used when rendering placeholder facet slots as empty subplots to avoid
    /// duplicate ownership labels on non-owner paths.
    pub hide_invalid_facet_path_axes: bool,
    /// Shared cache of facet scale precompute artifacts for the current evaluation run.
    pub(crate) facet_scale_precompute_store: Arc<FacetScalePrecomputeStore>,
    /// Internal facet runtime sizing mode used to dispatch canvas-fit vs fixed-subplot paths.
    pub(crate) facet_runtime_sizing_mode: FacetRuntimeSizingMode,
    /// Effective debug overlay toggle for layout bounds.
    pub(crate) debug_layout_lines: bool,
}

impl EvaluationContext {
    pub fn new(
        theme: Arc<Theme>,
        session_context: Arc<SessionContext>,
        params: IndexMap<String, ScalarValue>,
        facet_tree: Arc<EvaluatedFacetTree>,
    ) -> Self {
        Self {
            theme,
            session_context,
            params,
            facet_tree,
            hide_invalid_facet_path_axes: false,
            facet_scale_precompute_store: Arc::new(FacetScalePrecomputeStore::default()),
            facet_runtime_sizing_mode: FacetRuntimeSizingMode::CanvasFit,
            debug_layout_lines: false,
        }
    }

    /// Create a new context with different params, reusing other fields (cheap Arc clones)
    pub fn with_params(&self, params: IndexMap<String, ScalarValue>) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params,
            facet_tree: self.facet_tree.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_lines: self.debug_layout_lines,
        }
    }

    /// Create a new context with canvas dimensions added to params (for media queries)
    pub fn with_dimension_params(&self, width: f32, height: f32) -> Self {
        let mut params = self.params.clone();
        params.insert("width".to_string(), ScalarValue::Float32(Some(width)));
        params.insert("height".to_string(), ScalarValue::Float32(Some(height)));
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params,
            facet_tree: self.facet_tree.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_lines: self.debug_layout_lines,
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
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params,
            facet_tree: self.facet_tree.clone(),
            hide_invalid_facet_path_axes: hidden,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_lines: self.debug_layout_lines,
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
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params,
            facet_tree: self.facet_tree.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_lines: self.debug_layout_lines,
        }
    }

    pub(crate) fn with_facet_runtime_sizing_mode(&self, mode: FacetRuntimeSizingMode) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params: self.params.clone(),
            facet_tree: self.facet_tree.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: mode,
            debug_layout_lines: self.debug_layout_lines,
        }
    }

    pub(crate) fn facet_runtime_sizing_mode(&self) -> FacetRuntimeSizingMode {
        self.facet_runtime_sizing_mode
    }

    pub(crate) fn with_debug_layout_lines(&self, enabled: bool) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params: self.params.clone(),
            facet_tree: self.facet_tree.clone(),
            hide_invalid_facet_path_axes: self.hide_invalid_facet_path_axes,
            facet_scale_precompute_store: self.facet_scale_precompute_store.clone(),
            facet_runtime_sizing_mode: self.facet_runtime_sizing_mode,
            debug_layout_lines: enabled,
        }
    }

    pub(crate) fn debug_layout_lines_enabled(&self) -> bool {
        self.debug_layout_lines
    }

    pub(crate) fn facet_scale_precompute_store(&self) -> &Arc<FacetScalePrecomputeStore> {
        &self.facet_scale_precompute_store
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

    // Convenience accessors that delegate to inner structs

    /// Get the theme
    pub fn theme(&self) -> &Arc<Theme> {
        &self.eval.theme
    }

    /// Get the session context
    pub fn session_context(&self) -> &Arc<SessionContext> {
        &self.eval.session_context
    }

    /// Get the params
    pub fn params(&self) -> &IndexMap<String, ScalarValue> {
        &self.eval.params
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
        let mut context_with_params = context.clone();
        context_with_params.params.extend(self.eval.params.clone());
        self.eval.theme.query(&context_with_params, property)
    }

    /// Get font size with parameter support
    pub fn font_size(&self, context: &ThemeContext) -> Option<f32> {
        let mut context_with_params = context.clone();
        context_with_params.params.extend(self.eval.params.clone());
        self.eval.theme.font_size(&context_with_params)
    }
}
