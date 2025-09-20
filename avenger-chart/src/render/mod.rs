//! Rendering pipeline for avenger-chart
//!
//! This module bridges the high-level chart API with the low-level rendering components.

pub mod canvas;
pub mod context;
mod debug;
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
pub use types::{LayoutSolution, RenderResult};

use crate::coords::CoordinateSystem;
use crate::plot::Plot;

/// Renderer for converting Plot specifications to SceneGraph
pub struct PlotRenderer<'a, C: CoordinateSystem> {
    pub(crate) plot: &'a Plot<C>,
}

// All implementation blocks are now in their respective modules:
// - renderer.rs: Main render() method and orchestration
// - scales.rs: Scale building and configuration
// - marks.rs: Mark rendering and channel processing
// - legends.rs: Legend creation and management
// - titles.rs: Title and subtitle rendering
// - layout.rs: Layout computation
// - guide_marks.rs: Guide mark creation
// - debug.rs: Debug visualization utilities
// - guide.rs: Guide system for coordinate systems
