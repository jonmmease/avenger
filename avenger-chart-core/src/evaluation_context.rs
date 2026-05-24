use std::sync::Arc;

use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{Theme, ThemeContext, ThemeValue};

/// Public/base evaluation context for chart evaluation.
///
/// This owns the stable inputs that coordinate systems, marks, scales, legends,
/// and theme resolution can share without depending on the top-level layout
/// runtime.
#[derive(Clone)]
pub struct EvaluationContext {
    /// The theme to use for rendering.
    pub theme: Arc<Theme>,
    /// The DataFusion session context for DataFrame operations.
    pub session_context: Arc<SessionContext>,
    /// Parameter values for prepared statements and theme/media evaluation.
    pub params: IndexMap<String, ScalarValue>,
}

impl EvaluationContext {
    pub fn new(
        theme: Arc<Theme>,
        session_context: Arc<SessionContext>,
        params: IndexMap<String, ScalarValue>,
    ) -> Self {
        Self {
            theme,
            session_context,
            params,
        }
    }

    /// Get the theme.
    pub fn theme(&self) -> &Arc<Theme> {
        &self.theme
    }

    /// Get the DataFusion session context.
    pub fn session_context(&self) -> &Arc<SessionContext> {
        &self.session_context
    }

    /// Get runtime parameter values.
    pub fn params(&self) -> &IndexMap<String, ScalarValue> {
        &self.params
    }

    /// Create a new context with different params, reusing other fields.
    pub fn with_params(&self, params: IndexMap<String, ScalarValue>) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params,
        }
    }

    /// Create a new context with canvas dimensions added to params.
    pub fn with_dimension_params(&self, width: f32, height: f32) -> Self {
        let mut params = self.params.clone();
        params.insert("width".to_string(), ScalarValue::Float32(Some(width)));
        params.insert("height".to_string(), ScalarValue::Float32(Some(height)));
        self.with_params(params)
    }

    /// Query a theme property with the context's runtime params applied.
    pub fn query_theme(&self, context: &ThemeContext, property: &str) -> Option<ThemeValue> {
        let mut context_with_params = context.clone();
        context_with_params.params.extend(self.params.clone());
        self.theme.query(&context_with_params, property)
    }

    /// Get font size with the context's runtime params applied.
    pub fn font_size(&self, context: &ThemeContext) -> Option<f32> {
        let mut context_with_params = context.clone();
        context_with_params.params.extend(self.params.clone());
        self.theme.font_size(&context_with_params)
    }

    /// Resolve a mark default value with the context's runtime params applied.
    pub fn mark_default(&self, mark_type: &str, channel: &str) -> Option<ScalarValue> {
        self.theme.mark_default(mark_type, channel, &self.params)
    }
}
