//! Chart frame layout engine.
//!
//! Neutral layout primitives (geometry, grid solving, requirement
//! alignment) live in the `avenger-layout` crate and are re-exported here.
//! This module keeps the chart-specific layers: frame chrome solving, content
//! solving, sizing specs, and adapters between chart-core geometry and
//! `avenger-layout` geometry.

pub(crate) mod alignment;
mod chrome;
pub(crate) mod concat_grid;
mod content_solver;
pub(crate) mod declared_frame;
mod frame_solver;
mod info;
pub(crate) mod placement;
mod sizing;

pub(crate) use alignment::{
    AlignmentNode, RoundDeltas, SingletonPolicy, SkippedGroupReason, align_by,
};
pub use avenger_chart_core::BandPosition;
pub use avenger_chart_core::{
    EdgeSlabs, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameLayout, FrameSizingPolicy,
    LayoutBounds, OverflowSide, OwnedEdgeSlabs, Size2D,
};
pub(crate) use avenger_layout::{
    EdgeDemand, EdgeGrant, Edges, GridShape, GridSlot, Size, Spacing as TrackSpacing,
};
pub(crate) use chrome::FrameChrome;
pub use content_solver::{
    ChildFrameContentMeasurement, ChildFrameContentPlan, ChildFrameContentSolver,
    ContentAllocation, ContentDemand, ContentLayout, ContentLayoutSolver,
    SinglePlotContentMeasurement, SinglePlotContentPlan, SinglePlotContentSolver,
};
pub(crate) use frame_solver::{
    AvengerFrameLayoutSolver, FrameLayoutInput, apply_frame_side_slab, overflow_side_value,
    retarget_frame_layout_for_plot_area,
};
pub use info::LegendLayoutInfo;
pub(crate) use placement::{BoundaryDemand, EdgeTargets, Orientation};
pub use sizing::{
    CanvasConstraint, ChartResizeAxisPolicy, ChartResizeParams, ChartResizePolicy, LayoutSpec,
    Margins, PlotConstraint,
};
pub(crate) use sizing::{
    EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode, ResolvedLayoutDimensions, SizeMode,
};

/// Convert chart-core edge slabs into neutral layout edges.
pub(crate) fn layout_edges(slabs: EdgeSlabs) -> Edges<f32> {
    Edges::new(slabs.top, slabs.right, slabs.bottom, slabs.left)
}
