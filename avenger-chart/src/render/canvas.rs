//! Canvas extension trait for rendering plots

use crate::error::AvengerChartError;
use crate::plot::CompiledPlot;
use avenger_wgpu::canvas::{Canvas, PngCanvas};
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

/// Extension trait for Canvas to render Plot objects
#[allow(async_fn_in_trait)]
pub trait CanvasExt {
    /// Render a plot to this canvas with a SessionContext and optional parameters
    async fn render_plot(
        &mut self,
        plot: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, datafusion::common::ScalarValue>>,
    ) -> Result<(), AvengerChartError>;
}

// Implement CanvasExt for PngCanvas
impl CanvasExt for PngCanvas {
    async fn render_plot(
        &mut self,
        plot: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, datafusion::common::ScalarValue>>,
    ) -> Result<(), AvengerChartError> {
        // Render to scene graph using the provided SessionContext and parameters
        let render_result = plot.render(ctx, params).await?;

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
