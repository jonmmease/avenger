//! Chart frame layout engine.
//!
//! Neutral layout primitives (geometry, band/grid solvers, requirement
//! alignment) live in the `avenger-layout` crate and are re-exported here.
//! This module keeps the chart-specific layers: frame chrome solving, content
//! solving, sizing specs, and adapters between chart-core geometry and
//! `avenger-layout` geometry.

mod band_position;
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
pub(crate) use avenger_layout::{
    AlignmentNode, BandItem, BandSolution, BandSpacing, BoundaryDemand, ConvergenceTrace,
    EdgeDemand, EdgeTargets, Edges, GridItem, GridRequirements, GridShape, GridSlot, LayoutItem,
    LayoutNode, LayoutSlotContent, Orientation, PlacedBandItem, PlacedRegion, PlacementSolution,
    RoundDeltas, SingletonPolicy, Size, SkippedGroupReason, TrackSpacing, TreeEnvelopeKind, align,
    align_by, grid_requirements, solve_grid_requirements, tree_envelope_with,
    zero_edge_demands,
};
#[cfg(test)]
pub(crate) use avenger_layout::{merge_grid_requirements, total_edge_demands, tree_envelope};
pub use band_position::BandPositionIterator;
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

/// Convert chart-core edge slabs into neutral layout edges.
pub(crate) fn layout_edges(slabs: EdgeSlabs) -> Edges<f32> {
    Edges::new(slabs.top, slabs.right, slabs.bottom, slabs.left)
}

fn layout_rect(bounds: LayoutBounds) -> avenger_layout::Rect {
    avenger_layout::Rect::new(bounds.x, bounds.y, bounds.width, bounds.height)
}

/// Project a child-local component bound into the parent frame's coordinate
/// space.
pub(crate) fn project_child_rect(
    parent_content_origin: [f32; 2],
    child_render_origin: [f32; 2],
    child_plot_bounds: LayoutBounds,
    child_bounds: LayoutBounds,
) -> LayoutBounds {
    let rect = avenger_layout::project_rect(
        parent_content_origin,
        child_render_origin,
        layout_rect(child_plot_bounds),
        layout_rect(child_bounds),
    );
    LayoutBounds {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}
