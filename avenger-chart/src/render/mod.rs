//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

#[cfg(any(feature = "pdf", feature = "wgpu"))]
use std::sync::Arc;

#[cfg(any(feature = "pdf", feature = "wgpu"))]
use avenger_common::time::Instant;
#[cfg(any(feature = "pdf", feature = "wgpu"))]
use datafusion::{common::ScalarValue, prelude::SessionContext};
#[cfg(any(feature = "pdf", feature = "wgpu"))]
use indexmap::IndexMap;

#[cfg(any(feature = "pdf", feature = "wgpu"))]
use crate::{
    error::AvengerChartError,
    plot::{
        CompiledPlot, EvaluationRequest, NativeWidgetPlotId, NativeWidgetRuntimeResources,
        PlotSessionOptions,
    },
};

#[cfg(feature = "wgpu")]
pub mod canvas;
pub mod context;
pub mod debug;
#[cfg(feature = "pdf")]
pub mod pdf;
#[cfg(any(feature = "svg", feature = "pdf", feature = "doc-render"))]
pub mod resources;
#[cfg(feature = "svg")]
pub mod svg;
pub mod types;
#[cfg(feature = "wgpu")]
pub mod wgpu;

// Re-export commonly used types
#[cfg(feature = "image-resources")]
pub use avenger_image::{ImageResourceCache, ImageResourceLoadOptions, ImageResourceResolver};
#[cfg(feature = "wgpu")]
pub use canvas::CanvasExt;
pub use context::{EvaluationContext, RenderContext, RenderState};
#[cfg(feature = "pdf")]
pub use pdf::PdfRenderer;
#[cfg(feature = "svg")]
pub use svg::SvgRenderer;
pub use types::{
    CoordinationCheckpoint, EvaluatedChildFrameKind, EvaluatedChildFrameSegment,
    EvaluatedEventDatumRows, EvaluatedEventDatumState, EvaluatedInteractionScope,
    EvaluatedInteractionState, EvaluatedNativeWidgetAttachment, EvaluatedNativeWidgetState,
    EvaluatedPlot, EvaluatedWidgetFrame, EvaluatedWidgetFrameState, EvaluationMetrics,
    EvaluationMode, EvaluationOptions, FacetLayoutMetrics, FacetLayoutRefinement,
    FacetSubtreeCheckpoint, FacetSubtreeSelector, FacetSubtreeSnapshot, InteractionScopeId,
    InteractionScopeKind, LayoutDebugOverlayMode, LayoutSnapshot, LayoutSolution,
    LegendMeasurements, PreviewProfileFallbackReason, RefinementCheckpoint, WholeChartSnapshot,
    WidgetFrame, WidgetFrameAssignments,
};
#[cfg(feature = "wgpu")]
pub use wgpu::WgpuRenderer;

#[cfg(any(feature = "pdf", feature = "wgpu"))]
pub(crate) async fn evaluate_for_export(
    compiled: &CompiledPlot,
    ctx: &SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: EvaluationOptions,
    native_widgets: Option<&(NativeWidgetRuntimeResources, NativeWidgetPlotId)>,
) -> Result<EvaluatedPlot, AvengerChartError> {
    let Some((resources, plot_id)) = native_widgets else {
        return compiled.evaluate_with_options(ctx, params, options).await;
    };
    let mut session = Arc::new(compiled.clone()).instantiate(Arc::new(ctx.clone()));
    session.set_options(PlotSessionOptions::from_native_widget_resources(
        resources,
        plot_id.clone(),
    ));
    let mut request = EvaluationRequest::new().options(options).at(Instant::now());
    if let Some(params) = params {
        request = request.params(params);
    }
    let evaluated = session.evaluate(request).await;
    drop(session);
    let cleanup = resources.evict_plot(plot_id.clone(), Instant::now());
    match evaluated {
        Ok(evaluated) => {
            cleanup?;
            Ok(evaluated)
        }
        Err(error) => {
            let _ = cleanup;
            Err(error)
        }
    }
}
