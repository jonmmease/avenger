//! Plot module for data visualization
//!
//! This module provides the core `Plot` type for creating visualizations,
//! along with supporting types for titles, scales, and axes.

mod channel;
pub(crate) mod compiled;
mod plot;
mod scales;
mod specs;
mod title;

// Re-export core plot types
pub use crate::chart_core::IntoExpr;
pub use compiled::CompiledPlot;
pub use plot::Plot;

// Re-export title types
pub use title::{PlotSubtitle, PlotTitle, TitleAlign, TitleSpan};

// Re-export specification types
pub use specs::{AxisSpec, ScaleSpec};
