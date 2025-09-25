//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

pub mod canvas;
pub mod context;
pub mod debug;
mod guide;
mod guide_marks;
mod layout;
mod legends;
mod marks;
mod renderer;
mod scales;
mod titles;
pub mod types;

// Re-export commonly used types
pub use canvas::CanvasExt;
pub use context::RenderContext;
pub use legends::LegendMeasurements;
pub use types::{LayoutSolution, RenderResult};

use crate::coords::CoordinateSystem;
use crate::plot::Plot;

/// Renderer for converting Plot specifications to SceneGraph
pub struct PlotRenderer<'a, C: CoordinateSystem> {
    pub(crate) plot: &'a Plot<C>,
}
