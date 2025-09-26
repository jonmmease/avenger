//! Canvas extension trait for rendering plots

use crate::error::AvengerChartError;
use crate::plot::CompiledPlot;
use avenger_wgpu::canvas::{Canvas, PngCanvas};

/// Extension trait for Canvas to render Plot objects
#[allow(async_fn_in_trait)]
pub trait CanvasExt {
    /// Render a plot to this canvas
    async fn render_plot(&mut self, plot: &CompiledPlot) -> Result<(), AvengerChartError>;
}

// Implement CanvasExt for PngCanvas
impl CanvasExt for PngCanvas {
    async fn render_plot(&mut self, plot: &CompiledPlot) -> Result<(), AvengerChartError> {
        // Render to scene graph
        let render_result = plot.render().await?;

        // Pass scene graph to canvas for rendering
        self.set_scene(&render_result.scene_graph)
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))?;

        // Render to the canvas
        self.render()
            .await
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))?;

        Ok(())
    }
}
