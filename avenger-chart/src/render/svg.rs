//! SVG string renderer for Avenger charts.
//!
//! This module evaluates compiled plots and renders the resulting scene graph
//! through the `avenger-svg` backend.
//!
//! SVG `width`, `height`, and `viewBox` values are expressed in the same
//! logical chart pixels used by chart layout and PNG export. Tests or callers
//! that need PNG bytes can rasterize the returned SVG with `resvg`; expect small
//! antialiasing and text-rendering differences from the WGPU PNG path.

use std::{path::Path, sync::Arc};

use avenger_image::{ImageResourceCache, ImageResourceLoadOptions, ImageResourceResolver};
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{
    error::AvengerChartError,
    plot::CompiledPlot,
    render::{EvaluatedPlot, EvaluationOptions, resources::resolve_evaluated_plot_image_resources},
};

/// Renderer that exports evaluated plots as SVG strings.
///
/// ```rust,ignore
/// use avenger_chart::render::SvgRenderer;
///
/// let svg = SvgRenderer::new()
///     .render(&compiled, &ctx, None)
///     .await?;
/// std::fs::write("chart.svg", svg)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct SvgRenderer {
    scene_renderer: avenger_svg::SvgRenderer,
    image_resource_resolver: Arc<dyn ImageResourceResolver>,
    image_resource_load_options: ImageResourceLoadOptions,
}

impl Default for SvgRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SvgRenderer {
    /// Create a renderer with default SVG options.
    pub fn new() -> Self {
        let options = avenger_svg::SvgRenderOptions {
            font_resolution: crate::fonts::default_font_resolution(),
            ..Default::default()
        };
        Self {
            scene_renderer: avenger_svg::SvgRenderer::new().with_options(options),
            image_resource_resolver: Arc::new(ImageResourceCache::new()),
            image_resource_load_options: ImageResourceLoadOptions::default(),
        }
    }

    /// Override the SVG scenegraph renderer options.
    ///
    /// The chart's bundled default fonts stay registered alongside any
    /// caller-provided font resolution so theme text always resolves.
    pub fn with_options(mut self, mut options: avenger_svg::SvgRenderOptions) -> Self {
        options.font_resolution = crate::fonts::with_chart_font_defaults(options.font_resolution);
        self.scene_renderer = avenger_svg::SvgRenderer::new().with_options(options);
        self
    }

    pub fn with_image_resource_resolver(
        mut self,
        resolver: Arc<dyn ImageResourceResolver>,
    ) -> Self {
        self.image_resource_resolver = resolver;
        self
    }

    pub fn with_image_resource_load_options(mut self, options: ImageResourceLoadOptions) -> Self {
        self.image_resource_load_options = options;
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
        self.render_evaluated_plot(&evaluated_plot)
    }

    pub fn render_evaluated_plot(
        &self,
        evaluated_plot: &EvaluatedPlot,
    ) -> Result<String, AvengerChartError> {
        let scene_graph = resolve_evaluated_plot_image_resources(
            evaluated_plot,
            self.image_resource_resolver.as_ref(),
            self.image_resource_load_options,
        )?;
        self.scene_renderer
            .render_scene_graph(&scene_graph)
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
