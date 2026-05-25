pub use avenger_chart_core::{
    EdgeSlabs, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameLayout, FrameSizingPolicy,
    LayoutBounds, OverflowSide, OwnedEdgeSlabs, Size2D,
};

use avenger_chart_core::LegendPosition;

/// Types of components that can be laid out.
#[derive(Debug, Clone)]
pub(crate) enum ComponentType {
    PlotArea,
    GuideOverflow(OverflowSide),
    LegendContainer(LegendPosition),
    Title,
    Subtitle,
}

/// Minimum size in pixels for creating guide overflow regions.
/// Overflow regions smaller than this are ignored to avoid unnecessary grid complexity.
pub(crate) const MIN_GUIDE_OVERFLOW_SIZE: f32 = 2.0;
