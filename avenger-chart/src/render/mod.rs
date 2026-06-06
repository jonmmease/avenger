//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

pub mod canvas;
pub mod context;
pub mod debug;
pub mod types;
pub mod wgpu;

// Re-export commonly used types
pub use canvas::CanvasExt;
pub use context::{EvaluationContext, RenderContext, RenderState};
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
