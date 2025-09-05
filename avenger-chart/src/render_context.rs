//! Rendering context that carries theme through the rendering pipeline

use crate::theme::Theme;
use std::sync::Arc;

/// Context passed through the rendering pipeline
#[derive(Clone)]
pub struct RenderContext {
    /// The theme to use for rendering
    pub theme: Arc<Theme>,
}

impl RenderContext {
    pub fn new(theme: Theme) -> Self {
        Self {
            theme: Arc::new(theme),
        }
    }
}
