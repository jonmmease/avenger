//! Chart layout engine using Taffy flexbox

mod chart_layout;
mod grid;
mod info;
pub(crate) mod legend;
mod sizing;
mod types;

// Re-export main types
pub use chart_layout::ChartLayout;
pub use info::{merge_scale_updates, LayoutInfo, ScaleUpdates};
pub use sizing::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint};
pub(crate) use sizing::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode, SizeMode};
pub use types::{LayoutBounds, LayoutResult};
