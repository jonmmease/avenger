use datafusion::prelude::SessionContext;
use std::path::Path;

use crate::coords::CoordinateSystem;
use crate::plot::CompiledPlot;
use crate::prelude::*;
use crate::render::WgpuRenderer;

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
    let renderer = WgpuRenderer::new().with_scale(3.0);
    renderer
        .write_png(compiled, ctx, None, output)
        .await
        .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync + 'static>)
}
