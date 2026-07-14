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

use crate::{
    error::AvengerChartError,
    plot::{CompiledPlot, NativeWidgetPlotId, NativeWidgetRuntimeResources},
    render::{EvaluationOptions, evaluate_for_export},
};

/// Renderer that uses the WGPU backend to rasterize plots.
#[derive(Clone)]
pub struct WgpuRenderer {
    canvas_config: CanvasConfig,
    scale: f32,
    native_widgets: Option<(NativeWidgetRuntimeResources, NativeWidgetPlotId)>,
}

impl Default for WgpuRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl WgpuRenderer {
    /// Create a renderer with default settings (scale = 1.0).
    pub fn new() -> Self {
        let canvas_config = CanvasConfig {
            font_resolution: crate::fonts::default_font_resolution(),
            ..Default::default()
        };
        Self {
            canvas_config,
            scale: 1.0,
            native_widgets: None,
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

    /// Install native-widget runtime resources for one isolated export member.
    ///
    /// Live instances are detached and final-evicted after every render
    /// attempt, including failed evaluation.
    pub fn with_native_widget_runtime(
        mut self,
        resources: NativeWidgetRuntimeResources,
        plot_id: NativeWidgetPlotId,
    ) -> Self {
        self.native_widgets = Some((resources, plot_id));
        self
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
        let evaluated_plot =
            evaluate_for_export(compiled, ctx, params, options, self.native_widgets.as_ref())
                .await?;

        let dimensions = CanvasDimensions {
            size: [
                evaluated_plot.scene_graph.width,
                evaluated_plot.scene_graph.height,
            ],
            scale: self.scale,
        };

        let mut canvas = PngCanvas::new(dimensions, self.canvas_config.clone())
            .await
            .map_err(|err| AvengerChartError::InternalError(err.to_string()))?;
        canvas
            .set_scene(&evaluated_plot.scene_graph)
            .map_err(|err| AvengerChartError::InternalError(err.to_string()))?;

        let image = canvas
            .render()
            .await
            .map_err(|err| AvengerChartError::InternalError(err.to_string()))?;
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
    image
        .save(output)
        .map_err(|err| AvengerChartError::InternalError(err.to_string()))?;
    Ok(())
}
