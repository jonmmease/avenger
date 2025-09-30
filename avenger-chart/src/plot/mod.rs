//! Plot module for data visualization
//!
//! This module provides the core `Plot` type for creating visualizations,
//! along with supporting types for titles, scales, and axes.

mod channel;
mod compiled_plot;
mod compiled_plot_rendering;
mod plot;
mod scales;
mod specs;
mod title;

// Re-export core plot types
pub use compiled_plot::CompiledPlot;
pub use plot::Plot;

// Re-export title types
pub use title::{PlotSubtitle, PlotTitle, TitleAlign};

// Re-export specification types
pub use specs::{AxisSpec, ScaleSpec};
