//! Plot module for data visualization
//!
//! This module provides the core `Plot` type for creating visualizations,
//! along with supporting types for titles, scales, and axes.

mod channel;
mod plot;
mod scales;
mod specs;
mod title;

// Re-export core plot type
pub use plot::{Plot, SerializablePlotRenderer};

// Re-export title types
pub use title::{PlotSubtitle, PlotTitle, TitleAlign};

// Re-export specification types
pub use specs::{AxisSpec, ScaleSpec};
