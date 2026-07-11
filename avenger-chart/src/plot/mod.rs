//! Plot module for data visualization
//!
//! This module provides the core `Plot` type for creating visualizations,
//! along with supporting types for titles, scales, and axes.

mod channel;
mod chart;
pub(crate) mod compiled;
mod plot;
mod scales;
mod title;

// Re-export core plot types
pub use avenger_chart_core::IntoExpr;
pub use chart::Chart;
pub use compiled::{
    CompiledPlot, EvaluationRequest, PlotSession, PlotSessionOptions, ScopedParamAssignment,
    ScopedParamStoreSnapshot, ScopedStoreAssignment, SelectionAssignment, SelectionStateUpdate,
    StoreStateUpdate,
};
pub use plot::Plot;

// Re-export title types
pub use avenger_chart_core::{TitleAlign, TitleSpan};
pub use title::{PlotSubtitle, PlotTitle};

// Re-export specification types
pub use avenger_chart_core::AxisSpec;
pub use avenger_chart_scales::PlotScaleSpec as ScaleSpec;
