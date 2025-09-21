//! Chart layout engine using Taffy flexbox

mod chart_layout;
mod grid;
mod legend;
mod sizing;
mod text;
mod types;

// Re-export main types
pub use chart_layout::ChartLayout;
pub use sizing::{LayoutSpec, Margins, CanvasConstraint, PlotConstraint};
pub(crate) use sizing::SizeMode;
pub use types::{LayoutBounds, LayoutResult};
