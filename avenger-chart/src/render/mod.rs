//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

pub mod canvas;
pub mod context;
pub mod debug;
pub mod pdf;
pub mod resources;
pub mod svg;
pub mod types;
pub mod wgpu;

// Re-export commonly used types
pub use avenger_image::{ImageResourceCache, ImageResourceLoadOptions, ImageResourceResolver};
pub use canvas::CanvasExt;
pub use context::{EvaluationContext, RenderContext, RenderState};
pub use pdf::PdfRenderer;
pub use svg::SvgRenderer;
pub use types::{
    CoordinationCheckpoint, EvaluatedChildFrameKind, EvaluatedChildFrameSegment,
    EvaluatedEventDatumRows, EvaluatedEventDatumState, EvaluatedInteractionScope,
    EvaluatedInteractionState, EvaluatedPlot, EvaluationMetrics, EvaluationMode, EvaluationOptions,
    FacetLayoutMetrics, FacetLayoutRefinement, FacetSubtreeCheckpoint, FacetSubtreeSelector,
    FacetSubtreeSnapshot, InteractionScopeId, InteractionScopeKind, LayoutDebugOverlayMode,
    LayoutSnapshot, LayoutSolution, LegendMeasurements, PreviewProfileFallbackReason,
    RefinementCheckpoint, WholeChartSnapshot,
};
pub use wgpu::WgpuRenderer;
