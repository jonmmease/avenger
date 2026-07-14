//! PDF renderer for Avenger charts.
//!
//! This module evaluates compiled plots and renders the resulting scene graph
//! through the `avenger-pdf` backend.

use std::{path::Path, sync::Arc};

use avenger_image::{ImageResourceCache, ImageResourceLoadOptions, ImageResourceResolver};
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{
    error::AvengerChartError,
    plot::{CompiledPlot, NativeWidgetPlotId, NativeWidgetRuntimeResources},
    render::{
        EvaluatedPlot, EvaluationOptions, evaluate_for_export,
        resources::resolve_evaluated_plot_image_resources,
    },
};

/// Renderer that exports evaluated plots as PDF bytes.
#[derive(Clone)]
pub struct PdfRenderer {
    scene_renderer: avenger_pdf::PdfRenderer,
    image_resource_resolver: Arc<dyn ImageResourceResolver>,
    image_resource_load_options: ImageResourceLoadOptions,
    native_widgets: Option<(NativeWidgetRuntimeResources, NativeWidgetPlotId)>,
}

impl Default for PdfRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfRenderer {
    /// Create a renderer with default PDF options.
    pub fn new() -> Self {
        let options = avenger_pdf::PdfRenderOptions {
            font_resolution: crate::fonts::default_font_resolution(),
            ..Default::default()
        };
        Self {
            scene_renderer: avenger_pdf::PdfRenderer::new().with_options(options),
            image_resource_resolver: Arc::new(ImageResourceCache::new()),
            image_resource_load_options: ImageResourceLoadOptions::default(),
            native_widgets: None,
        }
    }

    /// Override the PDF scenegraph renderer options.
    ///
    /// The chart's bundled default fonts stay registered alongside any
    /// caller-provided font resolution so theme text always embeds.
    pub fn with_options(mut self, mut options: avenger_pdf::PdfRenderOptions) -> Self {
        options.font_resolution = crate::fonts::with_chart_font_defaults(options.font_resolution);
        self.scene_renderer = avenger_pdf::PdfRenderer::new().with_options(options);
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

    /// Render a compiled plot to PDF bytes.
    ///
    /// ```rust,ignore
    /// use avenger_chart::render::PdfRenderer;
    ///
    /// let pdf = PdfRenderer::new()
    ///     .render(&compiled, &ctx, None)
    ///     .await?;
    /// std::fs::write("chart.pdf", pdf)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub async fn render(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
    ) -> Result<Vec<u8>, AvengerChartError> {
        self.render_with_options(compiled, ctx, params, EvaluationOptions::default())
            .await
    }

    /// Render a compiled plot to PDF bytes with explicit evaluation options.
    pub async fn render_with_options(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
    ) -> Result<Vec<u8>, AvengerChartError> {
        let evaluated_plot =
            evaluate_for_export(compiled, ctx, params, options, self.native_widgets.as_ref())
                .await?;
        self.render_evaluated_plot(&evaluated_plot)
    }

    pub fn render_evaluated_plot(
        &self,
        evaluated_plot: &EvaluatedPlot,
    ) -> Result<Vec<u8>, AvengerChartError> {
        let scene_graph = resolve_evaluated_plot_image_resources(
            evaluated_plot,
            self.image_resource_resolver.as_ref(),
            self.image_resource_load_options,
        )?;
        self.scene_renderer
            .render_scene_graph(&scene_graph)
            .map_err(|err| AvengerChartError::InternalError(err.to_string()))
    }

    /// Write a compiled plot to a PDF file.
    ///
    /// ```rust,ignore
    /// use avenger_chart::render::PdfRenderer;
    ///
    /// PdfRenderer::new()
    ///     .write_pdf(&compiled, &ctx, None, "chart.pdf")
    ///     .await?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub async fn write_pdf<P: AsRef<Path>>(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        output: P,
    ) -> Result<(), AvengerChartError> {
        self.write_pdf_with_options(compiled, ctx, params, output, EvaluationOptions::default())
            .await
    }

    /// Write a compiled plot to a PDF file with explicit evaluation options.
    pub async fn write_pdf_with_options<P: AsRef<Path>>(
        &self,
        compiled: &CompiledPlot,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        output: P,
        options: EvaluationOptions,
    ) -> Result<(), AvengerChartError> {
        let pdf = self
            .render_with_options(compiled, ctx, params, options)
            .await?;
        save_pdf(&pdf, output)
    }
}

fn save_pdf<P: AsRef<Path>>(pdf: &[u8], output: P) -> Result<(), AvengerChartError> {
    let output = output.as_ref();
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| AvengerChartError::InternalError(err.to_string()))?;
    }
    std::fs::write(output, pdf).map_err(|err| AvengerChartError::InternalError(err.to_string()))?;
    Ok(())
}
