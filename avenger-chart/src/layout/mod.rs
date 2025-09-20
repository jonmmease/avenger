//! Chart layout engine using Taffy flexbox

mod chart_layout;
mod grid;
mod legend;
mod text;
mod types;

// Re-export main types
pub use chart_layout::ChartLayout;
pub use types::{LayoutBounds, LayoutResult};
