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

use avenger_layout::{
    EdgeDemand, Layout, Orientation, PlacedRegion, PlacementSolution, RegionDetail, Side, Size,
    SolveOptions, Spacing,
};

/// Cross-axis alignment for children inside a one-dimensional band.
///
/// Production placements are all `Start` today; the other variants complete
/// the policy (and are exercised by tests).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CrossAlign {
    #[default]
    Start,
    #[allow(dead_code)]
    Center,
    #[allow(dead_code)]
    End,
}

/// Main-axis rendered demand outside one child boundary
/// (leading/trailing edge totals projected onto the main axis).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoundaryDemand {
    pub before: f32,
    pub after: f32,
}

/// Sized child input for placement that owns its sibling gaps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandItem<Id = usize> {
    pub id: Id,
    pub main_size: f32,
    pub cross_size: f32,
    pub boundary: BoundaryDemand,
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
    /// Build from child sizes, sibling boundary demands, and spacing policy
    /// by solving a 1×N (or N×1) `Layout` row/column of measured leaves.
    pub fn solve(
        orientation: Orientation,
        items: &[BandItem<Id>],
        spacing: Spacing,
        cross_align: CrossAlign,
    ) -> Self {
        let leaves = items.iter().map(|child| {
            let (size, before_side, after_side) = match orientation {
                Orientation::Horizontal => (
                    Size::new(child.main_size, child.cross_size),
                    Side::Left,
                    Side::Right,
                ),
                Orientation::Vertical => (
                    Size::new(child.cross_size, child.main_size),
                    Side::Top,
                    Side::Bottom,
                ),
            };
            Layout::<usize>::leaf(size)
                .demand(before_side, EdgeDemand::total(child.boundary.before))
                .demand(after_side, EdgeDemand::total(child.boundary.after))
        });
        let band = match orientation {
            Orientation::Horizontal => Layout::row(leaves).column_spacing(spacing),
            Orientation::Vertical => Layout::column(leaves).row_spacing(spacing),
        };
        let solved = band
            .solve(&SolveOptions::default())
            .expect("a band of leaves always solves");

        let root = solved.at_path(&[]).expect("root region exists");
        let RegionDetail::Grid { tracks } = &root.detail else {
            unreachable!("a band root is a grid");
        };
        let (main_starts, main_sizes, cross_extent, main_extent) = match orientation {
            Orientation::Horizontal => (
                &tracks.column_starts,
                &tracks.column_sizes,
                tracks.row_sizes[0],
                root.content.width,
            ),
            Orientation::Vertical => (
                &tracks.row_starts,
                &tracks.row_sizes,
                tracks.column_sizes[0],
                root.content.height,
            ),
        };

        let placed_items = items
            .iter()
            .enumerate()
            .map(|(slot_index, child)| {
                let child_cross_size = child.cross_size.max(0.0);
                PlacedBandItem::with_cross_axis(
                    child.id.clone(),
                    main_starts[slot_index],
                    main_sizes[slot_index],
                    cross_align.offset(cross_extent, child_cross_size),
                    child_cross_size,
                )
            })
            .collect();

        let boundaries = items
            .iter()
            .enumerate()
            .map(|(slot_index, child)| {
                let granted = &solved
                    .at_path(&[slot_index])
                    .expect("band child region exists")
                    .granted;
                let (before, after) = match orientation {
                    Orientation::Horizontal => (granted.left.total, granted.right.total),
                    Orientation::Vertical => (granted.top.total, granted.bottom.total),
                };
                BandBoundary {
                    requested: child.boundary,
                    target: BoundaryDemand { before, after },
                }
            })
            .collect();

        Self {
            orientation,
            items: placed_items,
            boundaries,
            main_extent,
            cross_extent: Some(cross_extent),
        }
    }

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

impl CrossAlign {
    fn offset(self, available: f32, child: f32) -> f32 {
        let extra = (available - child).max(0.0);
        match self {
            Self::Start => 0.0,
            Self::Center => extra / 2.0,
            Self::End => extra,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: usize, main_size: f32, cross_size: f32, before: f32, after: f32) -> BandItem {
        BandItem {
            id,
            main_size,
            cross_size,
            boundary: BoundaryDemand { before, after },
        }
    }

    /// The Layout-backed solve is byte-identical to the legacy band solver
    /// for every placed float and boundary target.
    #[test]
    fn solve_matches_legacy_band_solver_exactly() {
        for orientation in [Orientation::Horizontal, Orientation::Vertical] {
            let items = [
                input(0, 60.0, 81.5, 5.25, 8.0),
                input(1, 90.1, 80.0, 14.0, 2.5),
                input(2, 45.0, 79.25, 1.0, 0.75),
            ];
            let spacing = Spacing {
                outer_start: 6.5,
                outer_end: 10.0,
                min_gap: 12.25,
            };
            let new = BandSolution::solve(orientation, &items, spacing, CrossAlign::Center);
            let legacy_items: Vec<avenger_layout::BandItem> = items
                .iter()
                .map(|item| avenger_layout::BandItem {
                    id: item.id,
                    main_size: item.main_size,
                    cross_size: item.cross_size,
                    boundary: avenger_layout::BoundaryDemand {
                        before: item.boundary.before,
                        after: item.boundary.after,
                    },
                })
                .collect();
            let legacy = avenger_layout::BandSolution::solve(
                orientation,
                &legacy_items,
                spacing,
                avenger_layout::CrossAlign::Center,
            );

            assert_eq!(new.main_extent, legacy.main_extent);
            assert_eq!(new.cross_extent, legacy.cross_extent);
            for (new_item, legacy_item) in new.items.iter().zip(legacy.items.iter()) {
                assert_eq!(new_item.main_start, legacy_item.main_start);
                assert_eq!(new_item.main_size, legacy_item.main_size);
                assert_eq!(new_item.cross_start, legacy_item.cross_start);
                assert_eq!(new_item.cross_size, legacy_item.cross_size);
            }
            for (new_boundary, legacy_boundary) in
                new.boundaries.iter().zip(legacy.boundaries.iter())
            {
                assert_eq!(
                    new_boundary.requested.before,
                    legacy_boundary.requested.before
                );
                assert_eq!(
                    new_boundary.requested.after,
                    legacy_boundary.requested.after
                );
                assert_eq!(new_boundary.target.before, legacy_boundary.target.before);
                assert_eq!(new_boundary.target.after, legacy_boundary.target.after);
            }
        }
    }

    #[test]
    fn horizontal_fixed_size_children_compute_expected_starts() {
        let placement = BandSolution::solve(
            Orientation::Horizontal,
            &[
                input(0, 30.0, 80.0, 0.0, 0.0),
                input(1, 40.0, 90.0, 0.0, 0.0),
            ],
            Spacing {
                outer_start: 5.0,
                outer_end: 7.0,
                min_gap: 10.0,
            },
            CrossAlign::default(),
        );

        assert_eq!(placement.items[0].main_start, 5.0);
        assert_eq!(placement.items[1].main_start, 45.0);
        assert_eq!(placement.main_extent, 92.0);
        assert_eq!(placement.cross_extent, Some(90.0));
    }

    #[test]
    fn boundary_demand_wins_when_larger_than_minimum_gap() {
        let placement = BandSolution::solve(
            Orientation::Horizontal,
            &[
                input(0, 20.0, 10.0, 0.0, 8.0),
                input(1, 20.0, 10.0, 7.0, 0.0),
            ],
            Spacing {
                min_gap: 4.0,
                ..Default::default()
            },
            CrossAlign::default(),
        );

        assert_eq!(placement.items[1].main_start, 35.0);
        assert_eq!(placement.main_extent, 55.0);
    }

    #[test]
    fn cross_alignment_offsets_smaller_children() {
        let placement = BandSolution::solve(
            Orientation::Horizontal,
            &[
                input(0, 30.0, 40.0, 0.0, 0.0),
                input(1, 30.0, 80.0, 0.0, 0.0),
            ],
            Spacing {
                min_gap: 5.0,
                ..Default::default()
            },
            CrossAlign::Center,
        );

        assert_eq!(placement.cross_extent, Some(80.0));
        assert_eq!(placement.items[0].cross_start, 20.0);
        assert_eq!(placement.items[1].cross_start, 0.0);

        let result = placement.to_placement_solution([0.0, 10.0], Size::new(1.0, 2.0));
        assert_eq!(result.child(0).unwrap().origin, [0.0, 30.0]);
        assert_eq!(result.child(1).unwrap().origin, [35.0, 10.0]);
    }
}
