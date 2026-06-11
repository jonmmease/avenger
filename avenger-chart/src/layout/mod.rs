//! Chart frame layout engine.
//!
//! Neutral layout primitives (geometry, band/grid solvers, requirement
//! alignment) live in the `avenger-layout` crate and are re-exported here.
//! This module keeps the chart-specific layers: frame chrome solving, content
//! solving, sizing specs, and adapters between chart-core geometry and
//! `avenger-layout` geometry.

pub(crate) mod alignment;
mod band;
mod band_position;
mod chrome;
mod content_solver;
pub(crate) mod declared_frame;
mod frame_solver;
mod info;
pub(crate) mod placement;
mod sizing;
mod uniform;

pub(crate) use alignment::{
    AlignmentNode, ConvergenceTrace, RoundDeltas, SingletonPolicy, SkippedGroupReason, align_by,
};
pub use avenger_chart_core::BandPosition;
pub use avenger_chart_core::{
    EdgeSlabs, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameLayout, FrameSizingPolicy,
    LayoutBounds, OverflowSide, OwnedEdgeSlabs, Size2D,
};
pub(crate) use avenger_layout::{
    EdgeDemand, Edges, GridItem, GridRequirements, GridShape, GridSlot, Orientation, Size,
    TrackSpacing,
};
pub(crate) use band::{BandItem, BandSolution, BoundaryDemand, CrossAlign, PlacedBandItem};
pub(crate) use placement::EdgeTargets;
pub(crate) use uniform::UniformTracks;

/// Chart placement metadata carried through the neutral handoff: the
/// concat retarget protocol state. `avenger-layout` never reads it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChartRegionMeta {
    pub(crate) content_size_override: Option<Size>,
    pub(crate) edge_targets: Option<EdgeTargets>,
}

/// The chart's placement handoff: positioned children with chart metadata.
pub type PlacementSolution = placement::PlacementSolution<usize, ChartRegionMeta>;
pub type PlacedRegion = placement::PlacedRegion<usize, ChartRegionMeta>;
pub use band_position::BandPositionIterator;
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
    let rect = placement::project_rect(
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
