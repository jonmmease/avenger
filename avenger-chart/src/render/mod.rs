//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

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
    EvaluatedInteractionState, EvaluatedPlot, EvaluatedWidgetFrame, EvaluatedWidgetFrameState,
    EvaluationMetrics, EvaluationMode, EvaluationOptions, FacetLayoutMetrics,
    FacetLayoutRefinement, FacetSubtreeCheckpoint, FacetSubtreeSelector, FacetSubtreeSnapshot,
    InteractionScopeId, InteractionScopeKind, LayoutDebugOverlayMode, LayoutSnapshot,
    LayoutSolution, LegendMeasurements, PreviewProfileFallbackReason, RefinementCheckpoint,
    WholeChartSnapshot,
};
#[cfg(feature = "wgpu")]
pub use wgpu::WgpuRenderer;
