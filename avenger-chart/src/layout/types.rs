//! Core types for chart layout

use std::collections::HashMap;

use indexmap::IndexMap;

use crate::{
    cartesian::axis::AxisPosition, coords::FacetAxis, guide::OverflowSpaceRequirement,
    legend::LegendPosition,
};

/// Realized component bounds for a chart frame.
#[derive(Debug, Clone)]
pub struct FrameLayout {
    pub plot_area: LayoutBounds,
    pub guide_overflows: HashMap<AxisPosition, LayoutBounds>,
    pub legends: IndexMap<String, LayoutBounds>,
    pub legends_by_position: IndexMap<LegendPosition, Vec<String>>,
    pub title: Option<LayoutBounds>,
    pub subtitle: Option<LayoutBounds>,
}

/// Backward-compatible alias while call sites migrate to frame terminology.
pub type LayoutResult = FrameLayout;

/// Bounding box for a layout component
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Two-dimensional size used by chart layout and measurement APIs.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size2D {
    pub width: f32,
    pub height: f32,
}

impl Size2D {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Side slabs around a frame or content rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeSlabs {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl EdgeSlabs {
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    pub fn max(self, other: Self) -> Self {
        Self {
            top: self.top.max(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
            left: self.left.max(other.left),
        }
    }

    pub fn min(self, other: Self) -> Self {
        Self {
            top: self.top.min(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
            left: self.left.min(other.left),
        }
    }

    pub fn side(self, side: OverflowSide) -> f32 {
        match side {
            OverflowSide::Top => self.top,
            OverflowSide::Right => self.right,
            OverflowSide::Bottom => self.bottom,
            OverflowSide::Left => self.left,
        }
    }

    pub fn set_side(&mut self, side: OverflowSide, value: f32) {
        match side {
            OverflowSide::Top => self.top = value,
            OverflowSide::Right => self.right = value,
            OverflowSide::Bottom => self.bottom = value,
            OverflowSide::Left => self.left = value,
        }
    }

    pub fn main_start(self, axis: FacetAxis) -> f32 {
        match axis {
            FacetAxis::Column => self.left,
            FacetAxis::Row => self.top,
        }
    }

    pub fn main_end(self, axis: FacetAxis) -> f32 {
        match axis {
            FacetAxis::Column => self.right,
            FacetAxis::Row => self.bottom,
        }
    }

    pub fn cross_start(self, axis: FacetAxis) -> f32 {
        match axis {
            FacetAxis::Column => self.top,
            FacetAxis::Row => self.left,
        }
    }

    pub fn cross_end(self, axis: FacetAxis) -> f32 {
        match axis {
            FacetAxis::Column => self.bottom,
            FacetAxis::Row => self.right,
        }
    }

    pub fn subtract_clamped(self, owned: OwnedEdgeSlabs) -> Self {
        Self {
            top: (self.top - owned.top).max(0.0),
            right: (self.right - owned.right).max(0.0),
            bottom: (self.bottom - owned.bottom).max(0.0),
            left: (self.left - owned.left).max(0.0),
        }
    }
}

impl From<OverflowSpaceRequirement> for EdgeSlabs {
    fn from(value: OverflowSpaceRequirement) -> Self {
        Self {
            top: value.top,
            right: value.right,
            bottom: value.bottom,
            left: value.left,
        }
    }
}

impl From<EdgeSlabs> for OverflowSpaceRequirement {
    fn from(value: EdgeSlabs) -> Self {
        Self {
            top: value.top,
            right: value.right,
            bottom: value.bottom,
            left: value.left,
        }
    }
}

/// Side slabs that a parent allocation already owns.
pub type OwnedEdgeSlabs = EdgeSlabs;

/// Per-dimension frame sizing owner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameDimensionSizing {
    CanvasConstrained { canvas_size: f32 },
    ContentSized { content_size: f32 },
    Auto,
}

/// Width/height sizing policy for a frame allocation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSizingPolicy {
    pub width: FrameDimensionSizing,
    pub height: FrameDimensionSizing,
}

/// Parent allocation granted to a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameAllocation {
    pub rect: LayoutBounds,
    pub sizing: FrameSizingPolicy,
    pub owned_slabs: OwnedEdgeSlabs,
}

/// Measured side demands for a frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameDemand {
    pub guide_slabs: EdgeSlabs,
    pub legend_slabs: EdgeSlabs,
    pub title_slabs: EdgeSlabs,
    pub rendered_envelope: EdgeSlabs,
    pub sibling_boundary_demand: EdgeSlabs,
}

impl FrameDemand {
    pub fn from_guide_and_rendered_envelope(
        guide: OverflowSpaceRequirement,
        rendered_envelope: OverflowSpaceRequirement,
    ) -> Self {
        let guide_slabs = EdgeSlabs::from(guide);
        let rendered_envelope = EdgeSlabs::from(rendered_envelope);
        let legend_slabs = rendered_envelope.subtract_clamped(guide_slabs);
        Self {
            guide_slabs,
            legend_slabs,
            rendered_envelope,
            ..Default::default()
        }
    }

    pub fn residual_overflow(self, owned_slabs: OwnedEdgeSlabs) -> EdgeSlabs {
        self.rendered_envelope.subtract_clamped(owned_slabs)
    }

    pub fn residual_overflow_after_owned_legend_slabs(
        self,
        owned_legend_slabs: OwnedEdgeSlabs,
    ) -> EdgeSlabs {
        self.residual_overflow(owned_legend_slabs.min(self.legend_slabs))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OverflowSide {
    Top,
    Right,
    Bottom,
    Left,
}

/// Types of components that can be laid out
#[derive(Debug, Clone)]
pub(crate) enum ComponentType {
    PlotArea,
    GuideOverflow(OverflowSide),
    LegendContainer(LegendPosition),
    Title,
    Subtitle,
}

/// Minimum size in pixels for creating guide overflow regions
/// Overflow regions smaller than this are ignored to avoid unnecessary grid complexity
pub(crate) const MIN_GUIDE_OVERFLOW_SIZE: f32 = 2.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_overflow_subtracts_owned_slabs_without_underflow() {
        let demand = FrameDemand {
            rendered_envelope: EdgeSlabs::new(10.0, 20.0, 30.0, 40.0),
            ..Default::default()
        };
        let owned = EdgeSlabs::new(3.0, 25.0, 0.0, 15.0);

        assert_eq!(
            demand.residual_overflow(owned),
            EdgeSlabs::new(7.0, 0.0, 30.0, 25.0)
        );
    }

    #[test]
    fn residual_overflow_after_owned_legend_slabs_preserves_guides() {
        let demand = FrameDemand::from_guide_and_rendered_envelope(
            OverflowSpaceRequirement {
                top: 10.0,
                right: 20.0,
                bottom: 30.0,
                left: 40.0,
            },
            OverflowSpaceRequirement {
                top: 15.0,
                right: 70.0,
                bottom: 45.0,
                left: 60.0,
            },
        );

        assert_eq!(demand.legend_slabs, EdgeSlabs::new(5.0, 50.0, 15.0, 20.0));
        assert_eq!(
            demand.residual_overflow_after_owned_legend_slabs(EdgeSlabs::new(
                100.0, 100.0, 100.0, 100.0
            )),
            EdgeSlabs::new(10.0, 20.0, 30.0, 40.0)
        );
    }

    #[test]
    fn edge_slabs_round_trip_overflow_requirement() {
        let overflow = OverflowSpaceRequirement {
            top: 1.0,
            right: 2.0,
            bottom: 3.0,
            left: 4.0,
        };

        let slabs = EdgeSlabs::from(overflow.clone());
        let round_trip = OverflowSpaceRequirement::from(slabs);

        assert_eq!(round_trip, overflow);
    }

    #[test]
    fn edge_slabs_side_and_axis_helpers_are_consistent() {
        let mut slabs = EdgeSlabs::new(1.0, 2.0, 3.0, 4.0);

        assert_eq!(slabs.side(OverflowSide::Left), 4.0);
        slabs.set_side(OverflowSide::Left, 5.0);
        assert_eq!(slabs.left, 5.0);

        assert_eq!(slabs.main_start(FacetAxis::Column), 5.0);
        assert_eq!(slabs.main_end(FacetAxis::Column), 2.0);
        assert_eq!(slabs.cross_start(FacetAxis::Column), 1.0);
        assert_eq!(slabs.cross_end(FacetAxis::Column), 3.0);

        assert_eq!(slabs.main_start(FacetAxis::Row), 1.0);
        assert_eq!(slabs.main_end(FacetAxis::Row), 3.0);
        assert_eq!(slabs.cross_start(FacetAxis::Row), 5.0);
        assert_eq!(slabs.cross_end(FacetAxis::Row), 2.0);
    }
}
