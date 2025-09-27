//! Rendering context that carries theme and dimensions through the rendering pipeline

use crate::theme::Theme;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

/// Context passed through the rendering pipeline
#[derive(Clone)]
pub struct RenderContext {
    /// The theme to use for rendering
    pub theme: Arc<dyn Theme>,
    /// Width of the plot area
    pub plot_width: f32,
    /// Height of the plot area
    pub plot_height: f32,
    /// The DataFusion session context for DataFrame operations
    pub session_context: Arc<SessionContext>,
}

impl RenderContext {
    pub fn new(
        theme: Arc<dyn Theme>,
        plot_width: f32,
        plot_height: f32,
        session_context: Arc<SessionContext>,
    ) -> Self {
        Self {
            theme,
            plot_width,
            plot_height,
            session_context,
        }
    }
}
