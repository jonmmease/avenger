//! Canvas extension trait for rendering plots

use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use avenger_wgpu::canvas::{Canvas, PngCanvas};

use crate::{error::AvengerChartError, plot::CompiledPlot};

/// Extension trait for Canvas to render Plot objects
#[allow(async_fn_in_trait)]
pub trait CanvasExt {
    /// Render a plot to this canvas with a SessionContext and optional parameters
    async fn render_plot(
        &mut self,
        plot: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
    ) -> Result<(), AvengerChartError>;
}

// Implement CanvasExt for PngCanvas
impl CanvasExt for PngCanvas {
    async fn render_plot(
        &mut self,
        plot: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
    ) -> Result<(), AvengerChartError> {
        // Evaluate to scene graph using the provided SessionContext and parameters
        let evaluated_plot = plot.evaluate(ctx, params).await?;

        // Pass scene graph to canvas for rendering
        self.set_scene(&evaluated_plot.scene_graph)
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))?;

        // Render to the canvas
        self.render()
            .await
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))?;

        Ok(())
    }
}
