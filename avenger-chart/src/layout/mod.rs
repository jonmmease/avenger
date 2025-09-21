//! Chart layout engine using Taffy flexbox

mod chart_layout;
mod grid;
pub(crate) mod legend;
mod sizing;
mod text;
mod types;

// Re-export main types
pub use chart_layout::ChartLayout;
pub(crate) use sizing::SizeMode;
pub use sizing::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint};
pub use types::{LayoutBounds, LayoutResult};
