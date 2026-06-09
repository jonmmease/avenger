//! Chart frame layout engine.

mod band_child_frame;
mod band_position;
mod child_frame;
mod content_solver;
mod frame_solver;
mod grid;
mod grid_alignment;
mod grid_tracks;
mod info;
mod sizing;

pub use avenger_chart_core::BandPosition;
pub use avenger_chart_core::{
    EdgeSlabs, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameLayout, FrameSizingPolicy,
    LayoutBounds, OverflowSide, OwnedEdgeSlabs, Size2D,
};
pub(crate) use band_child_frame::{
    BandItem, BandSolution, BandSpacing, BoundaryDemand, Orientation, PlacedBandItem,
};
pub use band_position::BandPositionIterator;
pub(crate) use child_frame::{EdgeTargets, PlacedRegion, PlacementSolution, project_child_rect};
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
pub(crate) use grid_alignment::{grid_content_delta, grid_edge_delta, merge_grid_requirements};
pub(crate) use grid_tracks::{
    EdgeDemand, GridItem, GridRequirements, GridShape, GridSlot, TrackSpacing, grid_requirements,
    solve_grid_requirements, zero_edge_demands,
};
#[cfg(test)]
pub(crate) use grid_tracks::{edge_demand_totals, span_axis_extent, total_edge_demands};
pub use info::LegendLayoutInfo;
pub use sizing::{
    CanvasConstraint, ChartResizeAxisPolicy, ChartResizePolicy, LayoutSpec, Margins, PlotConstraint,
};
pub(crate) use sizing::{
    EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode, ResolvedLayoutDimensions, SizeMode,
};
