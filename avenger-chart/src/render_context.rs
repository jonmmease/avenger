//! Rendering context that carries theme and other state through the rendering pipeline

use std::sync::Arc;
use crate::theme::Theme;

/// Context passed through the rendering pipeline
#[derive(Clone)]
pub struct RenderContext {
    /// The theme to use for rendering
    pub theme: Arc<Theme>,
    
    /// Plot dimensions (width, height)
    pub plot_dimensions: (f32, f32),
    
    /// DPI for text rendering
    pub dpi: f32,
    
    /// Current viewport for nested rendering contexts
    pub viewport: Option<Viewport>,
}

#[derive(Clone, Debug)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl RenderContext {
    pub fn new(theme: Theme, width: f32, height: f32, dpi: f32) -> Self {
        Self {
            theme: Arc::new(theme),
            plot_dimensions: (width, height),
            dpi,
            viewport: None,
        }
    }
    
    /// Create a sub-context with a specific viewport
    pub fn with_viewport(&self, viewport: Viewport) -> Self {
        let mut ctx = self.clone();
        ctx.viewport = Some(viewport);
        ctx
    }
    
    /// Get the effective width (viewport or plot width)
    pub fn width(&self) -> f32 {
        self.viewport.as_ref()
            .map(|v| v.width)
            .unwrap_or(self.plot_dimensions.0)
    }
    
    /// Get the effective height (viewport or plot height)
    pub fn height(&self) -> f32 {
        self.viewport.as_ref()
            .map(|v| v.height)
            .unwrap_or(self.plot_dimensions.1)
    }
}