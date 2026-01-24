//! Rendering context that carries theme and dimensions through the rendering pipeline

use crate::facet::computed_facet_spec::EvaluatedFacetTree;
use crate::facet::coordination::FacetCoordinationContext;
use crate::scales::ConfiguredScaleWithSpec;
use crate::theme::Theme;
use datafusion::common::ScalarValue;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

/// Context passed through the rendering pipeline
#[derive(Clone)]
pub struct RenderContext {
    /// The theme to use for rendering
    pub theme: Arc<Theme>,
    /// Width of the plot area
    pub plot_width: f32,
    /// Height of the plot area
    pub plot_height: f32,
    /// The DataFusion session context for DataFrame operations
    pub session_context: Arc<SessionContext>,
    /// Parameter values for prepared statements
    pub params: IndexMap<String, ScalarValue>,
    /// Configured scales available during rendering (coordinate + non-positional)
    pub scales: HashMap<String, ConfiguredScaleWithSpec>,
    /// Pre-computed facet structure for efficient domain lookups and visibility decisions.
    /// Built once at the start of evaluate() and shared throughout rendering.
    pub facet_spec: Arc<EvaluatedFacetTree>,
    /// Coordination context for nested facets.
    /// Carries domain information, guide ownership, and overflow coordination between facet levels.
    pub coordination_context: Option<FacetCoordinationContext>,
}

impl RenderContext {
    pub fn new(
        theme: Arc<Theme>,
        plot_width: f32,
        plot_height: f32,
        session_context: Arc<SessionContext>,
        params: IndexMap<String, ScalarValue>,
        scales: HashMap<String, ConfiguredScaleWithSpec>,
        facet_spec: Arc<EvaluatedFacetTree>,
    ) -> Self {
        Self {
            theme,
            plot_width,
            plot_height,
            session_context,
            params,
            scales,
            facet_spec,
            coordination_context: None,
        }
    }

    /// Set the pre-computed facet spec for efficient domain lookups.
    pub fn with_facet_spec(mut self, spec: Arc<EvaluatedFacetTree>) -> Self {
        self.facet_spec = spec;
        self
    }

    /// Get a reference to the facet spec.
    pub fn facet_spec(&self) -> &EvaluatedFacetTree {
        &self.facet_spec
    }

    /// Set the coordination context for nested facets.
    pub fn with_coordination_context(mut self, ctx: Option<FacetCoordinationContext>) -> Self {
        self.coordination_context = ctx;
        self
    }

    /// Get a reference to the coordination context.
    pub fn coordination_context(&self) -> Option<&FacetCoordinationContext> {
        self.coordination_context.as_ref()
    }

    /// Get a mutable reference to the coordination context.
    pub fn coordination_context_mut(&mut self) -> Option<&mut FacetCoordinationContext> {
        self.coordination_context.as_mut()
    }

    /// Take the coordination context, leaving None in its place.
    pub fn take_coordination_context(&mut self) -> Option<FacetCoordinationContext> {
        self.coordination_context.take()
    }

    /// Query theme property with automatic parameter resolution
    ///
    /// This is a convenience method that combines theme querying with parameter resolution.
    /// It resolves:
    /// - CSS variables (var()) using params or theme defaults
    /// - light-dark() functions using the "color-scheme" param
    ///
    /// # Arguments
    /// * `context` - The element context for CSS selector matching
    /// * `property` - The CSS property name
    ///
    /// # Returns
    /// Resolved ThemeValue if a matching rule is found, None otherwise
    ///
    /// # Example
    /// ```ignore
    /// use crate::theme::ThemeContext;
    ///
    /// let mark_ctx = ThemeContext::new("mark").with_subtype("rect");
    /// if let Some(fill) = render_ctx.query_theme(&mark_ctx, "fill") {
    ///     // Use resolved fill color
    /// }
    /// ```
    pub fn query_theme(
        &self,
        context: &crate::theme::ThemeContext,
        property: &str,
    ) -> Option<crate::theme::ThemeValue> {
        // Add render params to the context
        let mut context_with_params = context.clone();
        context_with_params.params.extend(self.params.clone());
        self.theme.query(&context_with_params, property)
    }

    /// Get font size with parameter support
    ///
    /// This resolves font sizes using params, including the "base-font-size" parameter
    /// which allows runtime control of base font size for rem calculations.
    ///
    /// # Arguments
    /// * `context` - The element context for CSS selector matching
    ///
    /// # Returns
    /// Resolved font size in pixels, or None if not found
    pub fn font_size(&self, context: &crate::theme::ThemeContext) -> Option<f32> {
        // Add render params to the context
        let mut context_with_params = context.clone();
        context_with_params.params.extend(self.params.clone());
        self.theme.font_size(&context_with_params)
    }
}
