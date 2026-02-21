//! Canvas extension trait for rendering plots

use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use avenger_wgpu::canvas::{Canvas, PngCanvas};

use crate::{error::AvengerChartError, plot::CompiledPlot, render::EvaluationOptions};

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

    /// Render a plot with explicit evaluation options.
    ///
    /// The default implementation delegates to `render_plot` to preserve compatibility
    /// for implementers that do not override this method.
    async fn render_plot_with_options(
        &mut self,
        plot: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        _options: EvaluationOptions,
    ) -> Result<(), AvengerChartError> {
        self.render_plot(plot, ctx, params).await
    }
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

    async fn render_plot_with_options(
        &mut self,
        plot: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
    ) -> Result<(), AvengerChartError> {
        let evaluated_plot = plot.evaluate_with_options(ctx, params, options).await?;

        self.set_scene(&evaluated_plot.scene_graph)
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))?;

        self.render()
            .await
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))?;

        Ok(())
    }
}
