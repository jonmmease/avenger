//! Chart frame layout engine.

mod chart_layout;
mod frame_solver;
mod grid;
mod info;
pub(crate) mod legend;
mod sizing;
mod types;

pub(crate) use frame_solver::{
    FrameLayoutInput, TaffyFrameLayoutSolver, apply_frame_side_slab, overflow_side_value,
    retarget_frame_layout_for_plot_area,
};
pub use info::LegendLayoutInfo;
pub use sizing::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint};
pub(crate) use sizing::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode, SizeMode};
pub use types::{
    EdgeSlabs, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameLayout, FrameSizingPolicy,
    LayoutBounds, LayoutResult, OwnedEdgeSlabs, Size2D,
};
