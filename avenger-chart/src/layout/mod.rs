//! Chart frame layout engine.

mod chart_layout;
mod content_solver;
mod frame_solver;
mod grid;
mod info;
pub(crate) mod legend;
mod sizing;
mod types;

pub use content_solver::{
    ContentAllocation, ContentCoordinationPlan, ContentDemand, ContentLayout, ContentLayoutSolver,
    FacetBandContentMeasurement, FacetBandContentPlan, FacetBandContentSolver,
    SinglePlotContentMeasurement, SinglePlotContentPlan, SinglePlotContentSolver,
};
pub(crate) use frame_solver::{
    FrameLayoutInput, TaffyFrameLayoutSolver, apply_frame_side_slab, overflow_side_value,
    retarget_frame_layout_for_plot_area,
};
pub use info::LegendLayoutInfo;
pub use sizing::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint};
pub(crate) use sizing::{
    EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode, ResolvedLayoutDimensions, SizeMode,
};
pub use types::{
    EdgeSlabs, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameLayout, FrameSizingPolicy,
    LayoutBounds, LayoutResult, OwnedEdgeSlabs, Size2D,
};
