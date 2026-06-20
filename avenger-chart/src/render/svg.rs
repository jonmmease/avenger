//! SVG string renderer for Avenger charts.
//!
//! This module evaluates compiled plots and renders the resulting scene graph
//! through the `avenger-svg` backend.

use std::path::Path;

use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{error::AvengerChartError, plot::CompiledPlot, render::EvaluationOptions};

/// Renderer that exports evaluated plots as SVG strings.
#[derive(Clone)]
pub struct SvgRenderer {
    scene_renderer: avenger_svg::SvgRenderer,
}

impl Default for SvgRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SvgRenderer {
    /// Create a renderer with default SVG options.
    pub fn new() -> Self {
        Self {
            scene_renderer: avenger_svg::SvgRenderer::new(),
        }
    }

    /// Override the SVG scenegraph renderer options.
    pub fn with_options(mut self, options: avenger_svg::SvgRenderOptions) -> Self {
        self.scene_renderer = avenger_svg::SvgRenderer::new().with_options(options);
        self
    }

    /// Render a compiled plot to an SVG string.
    pub async fn render(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
    ) -> Result<String, AvengerChartError> {
        self.render_with_options(compiled, ctx, params, EvaluationOptions::default())
            .await
    }

    /// Render a compiled plot to an SVG string with explicit evaluation options.
    pub async fn render_with_options(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
    ) -> Result<String, AvengerChartError> {
        let evaluated_plot = compiled.evaluate_with_options(ctx, params, options).await?;
        self.scene_renderer
            .render_scene_graph(&evaluated_plot.scene_graph)
            .map_err(|err| AvengerChartError::InternalError(err.to_string()))
    }

    /// Render a compiled plot directly to an SVG file.
    pub async fn write_svg<P: AsRef<Path>>(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        output: P,
    ) -> Result<(), AvengerChartError> {
        self.write_svg_with_options(compiled, ctx, params, output, EvaluationOptions::default())
            .await
    }

    /// Render a compiled plot directly to an SVG file with explicit evaluation options.
    pub async fn write_svg_with_options<P: AsRef<Path>>(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        output: P,
        options: EvaluationOptions,
    ) -> Result<(), AvengerChartError> {
        let svg = self
            .render_with_options(compiled, ctx, params, options)
            .await?;
        save_svg(&svg, output)
    }
}

fn save_svg<P: AsRef<Path>>(svg: &str, output: P) -> Result<(), AvengerChartError> {
    let output = output.as_ref();
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| AvengerChartError::InternalError(err.to_string()))?;
    }
    std::fs::write(output, svg).map_err(|err| AvengerChartError::InternalError(err.to_string()))?;
    Ok(())
}
