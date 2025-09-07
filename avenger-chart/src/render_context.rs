//! Rendering context that carries theme and dimensions through the rendering pipeline

use crate::theme::Theme;
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
}

impl RenderContext {
    pub fn new(theme: Theme, plot_width: f32, plot_height: f32) -> Self {
        Self {
            theme: Arc::new(theme),
            plot_width,
            plot_height,
        }
    }
}
