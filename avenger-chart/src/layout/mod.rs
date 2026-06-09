//! Chart frame layout engine.

mod band_child_frame;
mod band_position;
mod child_frame;
mod content_solver;
mod frame_solver;
mod grid;
mod info;
mod sizing;

pub use avenger_chart_core::BandPosition;
pub use avenger_chart_core::{
    EdgeSlabs, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameLayout, FrameSizingPolicy,
    LayoutBounds, OverflowSide, OwnedEdgeSlabs, Size2D,
};
pub(crate) use band_child_frame::{
    BandChildFrameInput, BandChildFramePlacement, BandDirection, BandPlacedChild, BandSpacing,
    BoundaryDemand1D,
};
pub use band_position::BandPositionIterator;
pub(crate) use child_frame::{
    ChildFramePlacementResult, ChildFrameRenderPlacement, ChildFrameSideSlabTargets,
    project_child_frame_bounds,
};
pub use content_solver::{
    ChildFrameContentMeasurement, ChildFrameContentPlan, ChildFrameContentSolver,
    ContentAllocation, ContentDemand, ContentLayout, ContentLayoutSolver,
    SinglePlotContentMeasurement, SinglePlotContentPlan, SinglePlotContentSolver,
};
pub(crate) use frame_solver::{
    AvengerFrameLayoutSolver, FrameLayoutInput, apply_frame_side_slab, overflow_side_value,
    retarget_frame_layout_for_plot_area,
};
pub(crate) use grid::{ComponentType, MIN_GUIDE_OVERFLOW_SIZE};
pub use info::LegendLayoutInfo;
pub use sizing::{
    CanvasConstraint, ChartResizeAxisPolicy, ChartResizePolicy, LayoutSpec, Margins, PlotConstraint,
};
pub(crate) use sizing::{
    EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode, ResolvedLayoutDimensions, SizeMode,
};
