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
pub use context::RenderContext;
pub use types::{LayoutSolution, LegendMeasurements, RenderResult};
pub use wgpu::WgpuRenderer;
