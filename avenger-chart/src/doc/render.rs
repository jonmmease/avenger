use datafusion::prelude::SessionContext;
use std::path::Path;

use crate::coords::CoordinateSystem;
use crate::plot::CompiledPlot;
use crate::prelude::*;
use crate::render::WgpuRenderer;
use crate::render::types::EvaluatedPlot;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, PngCanvas};

/// Render a plot directly to a PNG file using the WGPU backend.
pub async fn render_plot_to_png<C: CoordinateSystem + 'static>(
    ctx: &SessionContext,
    plot: Plot<C>,
    output: impl AsRef<Path>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let compiled = plot.compile(ctx).await?;
    render_compiled_plot_to_png(&compiled, ctx, output).await
}

/// Render a compiled plot to PNG, writing to the provided path.
pub async fn render_compiled_plot_to_png(
    compiled: &CompiledPlot,
    ctx: &SessionContext,
    output: impl AsRef<Path>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let renderer = WgpuRenderer::new().with_scale(4.0);
    renderer
        .write_png(compiled, ctx, None, output)
        .await
        .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync + 'static>)
}

/// Render an EvaluatedPlot directly to PNG, writing to the provided path.
///
/// This is useful for rendering multiple parameter variations from a single compiled plot:
/// ```ignore
/// let compiled = plot.compile(&ctx).await?;
/// let result1 = compiled.evaluate(&ctx, None).await?;
/// let result2 = compiled.evaluate(&ctx, Some(params)).await?;
/// render_evaluated_plot_to_png(&result1, "output1.png").await?;
/// render_evaluated_plot_to_png(&result2, "output2.png").await?;
/// ```
pub async fn render_evaluated_plot_to_png(
    result: &EvaluatedPlot,
    output: impl AsRef<Path>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let dimensions = CanvasDimensions {
        size: [result.scene_graph.width, result.scene_graph.height],
        scale: 4.0,
    };

    let mut canvas = PngCanvas::new(dimensions, Default::default()).await?;
    canvas
        .set_scene(&result.scene_graph)
        .map_err(|err| format!("Failed to set scene: {}", err))?;

    let image = canvas.render().await?;

    let output_path = output.as_ref();
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image.save(output_path)?;

    Ok(())
}
