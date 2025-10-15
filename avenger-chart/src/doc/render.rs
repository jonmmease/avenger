use std::path::Path;

use crate::render::types::EvaluatedPlot;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, PngCanvas};

/// Render an EvaluatedPlot directly to PNG, writing to the provided path.
///
/// This is the primary function for rendering plots in documentation examples.
/// It renders at 4x scale for crisp output.
///
/// # Example
/// ```ignore
/// let compiled = plot.compile(&ctx).await?;
/// let evaluated = compiled.evaluate(&ctx, None).await?;
/// render_evaluated_plot_to_png(&evaluated, "output.png").await?;
/// ```
///
/// For multiple parameter variations:
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
