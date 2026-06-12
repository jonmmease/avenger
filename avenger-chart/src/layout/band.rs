//! Chart-owned one-dimensional band placement, solved through the unified
//! `avenger_layout::Layout` API.
//!
//! A band is the one-track-cross-axis special case: each child occupies one
//! main-axis track, sibling boundary demands map to track edge demands
//! through the shared gap rule `max(min_gap, after + before)`, and
//! cross-axis alignment stays in this layer. The chart owns these types
//! because `BandSolution` doubles as a placement record across coordination
//! rounds (`from_positioned_children` reconstructs one from realized
//! positions).
//!
//! Byte-stability: concat's placement change detection compares these f32s
//! with `PartialEq`, so `solve` reads the solver's exact track floats
//! (relative `SolvedTracks` starts, per-child granted edges) and never
//! re-derives positions through float round trips.

use avenger_layout::Size;

/// Flow direction for a one-dimensional band arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

use super::placement::{PlacedRegion, PlacementSolution};

/// Main-axis rendered demand outside one child boundary
/// (leading/trailing edge totals projected onto the main axis).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoundaryDemand {
    pub before: f32,
    pub after: f32,
}

/// Positioned child in main/cross-axis coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedBandItem<Id = usize> {
    pub id: Id,
    pub main_start: f32,
    pub main_size: f32,
    pub cross_start: f32,
    pub cross_size: f32,
}

impl<Id> PlacedBandItem<Id> {
    pub fn new(id: Id, main_start: f32, main_size: f32) -> Self {
        Self {
            id,
            main_start,
            main_size,
            cross_start: 0.0,
            cross_size: 0.0,
        }
    }

    pub fn with_cross_axis(
        id: Id,
        main_start: f32,
        main_size: f32,
        cross_start: f32,
        cross_size: f32,
    ) -> Self {
        Self {
            id,
            main_start,
            main_size,
            cross_start: cross_start.max(0.0),
            cross_size: cross_size.max(0.0),
        }
    }
}

/// One child's main-axis overflow through a band solve: requested boundary
/// demand and the coordinated per-track target the solve produced.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BandBoundary {
    pub requested: BoundaryDemand,
    pub target: BoundaryDemand,
}

/// Placement result for a one-dimensional band of items.
#[derive(Debug, Clone, PartialEq)]
pub struct BandSolution<Id = usize> {
    pub orientation: Orientation,
    pub items: Vec<PlacedBandItem<Id>>,
    /// Per-item main-axis overflow, aligned with `items`. The first item's
    /// `before` and the last item's `after` are the band's own outward
    /// overflow (excluded from `main_extent`); interior boundaries are
    /// consumed by the gaps.
    pub boundaries: Vec<BandBoundary>,
    pub main_extent: f32,
    pub cross_extent: Option<f32>,
}

impl<Id: Clone> BandSolution<Id> {
    /// Convert main/cross-axis child placement into render-space origins
    /// relative to the parent content rectangle.
    pub fn to_placement_solution(
        &self,
        origin_offset: [f32; 2],
        fallback_content_size: Size,
    ) -> PlacementSolution<Id> {
        let placements = self
            .items
            .iter()
            .map(|child| {
                let origin = match self.orientation {
                    Orientation::Horizontal => [
                        child.main_start + origin_offset[0],
                        child.cross_start + origin_offset[1],
                    ],
                    Orientation::Vertical => [
                        child.cross_start + origin_offset[0],
                        child.main_start + origin_offset[1],
                    ],
                };
                PlacedRegion::new(child.id.clone(), origin)
            })
            .collect();

        let content_size = match self.orientation {
            Orientation::Horizontal => Size::new(
                self.main_extent,
                self.cross_extent.unwrap_or(fallback_content_size.height),
            ),
            Orientation::Vertical => Size::new(
                self.cross_extent.unwrap_or(fallback_content_size.width),
                self.main_extent,
            ),
        };

        PlacementSolution::new(content_size, placements)
    }
}
