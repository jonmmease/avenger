//! Rendering context that carries theme and dimensions through the rendering pipeline
//!
//! The context is split into three parts:
//! - `EvaluationContext` - Constant across entire evaluate() call, built once at top level
//! - `RenderState` - Changes per subplot, contains computed dimensions and scales
//! - `RenderContext` - Thin facade combining both for mark rendering API

use crate::coords::CoordMeasurement;
use crate::facet::evaluated_facet_tree::EvaluatedFacetTree;
use crate::scales::ConfiguredScaleWithSpec;
use crate::theme::Theme;
use datafusion::common::ScalarValue;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

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
        }
    }

    /// Create a new context with different params, reusing other fields (cheap Arc clones)
    pub fn with_params(&self, params: IndexMap<String, ScalarValue>) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params,
            facet_tree: self.facet_tree.clone(),
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
        }
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
    pub facet_path: &'a [datafusion::common::ScalarValue],
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
        facet_path: &'a [datafusion::common::ScalarValue],
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
    pub fn query_theme(
        &self,
        context: &crate::theme::ThemeContext,
        property: &str,
    ) -> Option<crate::theme::ThemeValue> {
        let mut context_with_params = context.clone();
        context_with_params.params.extend(self.eval.params.clone());
        self.eval.theme.query(&context_with_params, property)
    }

    /// Get font size with parameter support
    pub fn font_size(&self, context: &crate::theme::ThemeContext) -> Option<f32> {
        let mut context_with_params = context.clone();
        context_with_params.params.extend(self.eval.params.clone());
        self.eval.theme.font_size(&context_with_params)
    }
}
