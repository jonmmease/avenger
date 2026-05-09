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
    EvaluatedPlot, EvaluationMetrics, EvaluationOptions, FacetLayoutMetrics, LayoutSnapshot,
    LayoutSolution, LegendMeasurements,
};
pub use wgpu::WgpuRenderer;
