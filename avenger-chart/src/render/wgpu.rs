//! WGPU-based renderer for Avenger charts.
//!
//! This module provides a reusable renderer that can turn compiled plots into
//! images or PNG files using the `avenger-wgpu` backend. The renderer keeps
//! configuration such as the output scale and canvas options, making it easier
//! to share across renders.

use std::path::Path;

use datafusion::{common::ScalarValue, prelude::SessionContext};
use image::RgbaImage;
use indexmap::IndexMap;

use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

use crate::{error::AvengerChartError, plot::CompiledPlot, render::EvaluationOptions};

/// Renderer that uses the WGPU backend to rasterize plots.
#[derive(Clone)]
pub struct WgpuRenderer {
    canvas_config: CanvasConfig,
    scale: f32,
}

impl WgpuRenderer {
    /// Create a renderer with default settings (scale = 1.0).
    pub fn new() -> Self {
        Self {
            canvas_config: CanvasConfig::default(),
            scale: 1.0,
        }
    }

    /// Override the canvas configuration used when constructing `PngCanvas`.
    pub fn with_canvas_config(mut self, canvas_config: CanvasConfig) -> Self {
        self.canvas_config = canvas_config;
        self
    }

    /// Set the pixel scale applied to the logical scene dimensions.
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Return the scale used by this renderer.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Render a compiled plot to an in-memory `RgbaImage`.
    pub async fn render(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
    ) -> Result<RgbaImage, AvengerChartError> {
        self.render_with_options(compiled, ctx, params, EvaluationOptions::default())
            .await
    }

    /// Render a compiled plot to an in-memory `RgbaImage` with explicit evaluation options.
    pub async fn render_with_options(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
    ) -> Result<RgbaImage, AvengerChartError> {
        let evaluated_plot = compiled.evaluate_with_options(ctx, params, options).await?;

        let dimensions = CanvasDimensions {
            size: [
                evaluated_plot.scene_graph.width,
                evaluated_plot.scene_graph.height,
            ],
            scale: self.scale,
        };

        let mut canvas = PngCanvas::new(dimensions, self.canvas_config.clone()).await?;
        canvas
            .set_scene(&evaluated_plot.scene_graph)
            .map_err(|err| AvengerChartError::InternalError(err.to_string()))?;

        let image = canvas.render().await?;
        Ok(image)
    }

    /// Render a compiled plot directly to a PNG file.
    pub async fn write_png<P: AsRef<Path>>(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        output: P,
    ) -> Result<(), AvengerChartError> {
        self.write_png_with_options(compiled, ctx, params, output, EvaluationOptions::default())
            .await
    }

    /// Render a compiled plot directly to a PNG file with explicit evaluation options.
    pub async fn write_png_with_options<P: AsRef<Path>>(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        output: P,
        options: EvaluationOptions,
    ) -> Result<(), AvengerChartError> {
        let image = self
            .render_with_options(compiled, ctx, params, options)
            .await?;
        save_png(image, output)
    }
}

fn save_png<P: AsRef<Path>>(image: RgbaImage, output: P) -> Result<(), AvengerChartError> {
    let output = output.as_ref();
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| AvengerChartError::InternalError(err.to_string()))?;
    }
    image.save(output)?;
    Ok(())
}
