//! One-dimensional band placement.
//!
//! A band is the one-track-cross-axis special case of the grid solver: each
//! child occupies one main-axis track, sibling boundary demands map to track
//! edge demands, and the minimum inner gap maps to the main-axis
//! [`TrackSpacing::min_gap`]. Cross-axis alignment stays in this band layer.

use crate::geometry::{Edges, Orientation, Size};
use crate::grid::{
    GridItem, GridShape, GridSlot, TrackSpacing, grid_requirements, solve_grid_requirements,
};

use crate::region::{PlacedRegion, PlacementSolution};

/// Cross-axis alignment for children inside a one-dimensional band.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CrossAlign {
    #[default]
    Start,
    Center,
    End,
}

/// Main-axis rendered demand outside one child boundary.
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

/// Placement result for a one-dimensional band of children.
#[derive(Debug, Clone, PartialEq)]
pub struct BandSolution<Id = usize> {
    pub direction: Orientation,
    pub children: Vec<PlacedBandItem<Id>>,
    pub main_extent: f32,
    pub cross_extent: Option<f32>,
}

impl<Id> BandSolution<Id> {
    /// Build from child positions that have already been resolved by another
    /// system, such as a configured band scale.
    pub fn from_positioned_children(
        direction: Orientation,
        children: Vec<PlacedBandItem<Id>>,
        main_extent: f32,
        cross_extent: Option<f32>,
    ) -> Self {
        let cross_extent = cross_extent.or_else(|| {
            if children.is_empty() {
                None
            } else {
                Some(
                    children
                        .iter()
                        .map(|child| child.cross_start + child.cross_size)
                        .fold(0.0f32, f32::max),
                )
            }
        });

        Self {
            direction,
            children,
            main_extent,
            cross_extent,
        }
    }
}

impl<Id: Clone> BandSolution<Id> {
    /// Build from child sizes, sibling boundary demands, and spacing policy.
    ///
    /// Solved as a 1xN (or Nx1) grid: `spacing` is the main-axis
    /// [`TrackSpacing`] (gaps follow the grid rule
    /// `max(min_gap, after + before)`); `cross_align` positions children
    /// within the band's cross extent.
    pub fn from_sized_children(
        direction: Orientation,
        children: &[BandItem<Id>],
        spacing: TrackSpacing,
        cross_align: CrossAlign,
    ) -> Self {
        let shape = match direction {
            Orientation::Horizontal => GridShape {
                rows: 1,
                columns: children.len(),
            },
            Orientation::Vertical => GridShape {
                rows: children.len(),
                columns: 1,
            },
        };
        let items = children
            .iter()
            .enumerate()
            .map(|(slot_index, child)| {
                let (slot, content_size, total_edges) = match direction {
                    Orientation::Horizontal => (
                        GridSlot {
                            row: 0,
                            column: slot_index,
                            row_span: 1,
                            column_span: 1,
                        },
                        Size::new(child.main_size, child.cross_size),
                        Edges::new(0.0, child.boundary.after, 0.0, child.boundary.before),
                    ),
                    Orientation::Vertical => (
                        GridSlot {
                            row: slot_index,
                            column: 0,
                            row_span: 1,
                            column_span: 1,
                        },
                        Size::new(child.cross_size, child.main_size),
                        Edges::new(child.boundary.before, 0.0, child.boundary.after, 0.0),
                    ),
                };
                GridItem {
                    id: child.id.clone(),
                    slot,
                    content_size,
                    inner_edges: Edges::default(),
                    outer_edges: Edges::default(),
                    total_edges,
                }
            })
            .collect::<Vec<_>>();

        let mut requirements = grid_requirements(shape, Size::default(), &items)
            .expect("band slots are single-span and indexed within the band grid shape");
        match direction {
            Orientation::Horizontal => requirements.column_spacing = spacing,
            Orientation::Vertical => requirements.row_spacing = spacing,
        }
        let solution = solve_grid_requirements(&requirements, &items);

        let (main_starts, main_sizes, cross_extent, main_extent) = match direction {
            Orientation::Horizontal => (
                &solution.column_starts,
                &solution.column_widths,
                solution.row_heights[0],
                solution.content_size.width,
            ),
            Orientation::Vertical => (
                &solution.row_starts,
                &solution.row_heights,
                solution.column_widths[0],
                solution.content_size.height,
            ),
        };

        let placed_children = children
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

        Self {
            direction,
            children: placed_children,
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
            .children
            .iter()
            .map(|child| {
                let origin = match self.direction {
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

        let content_size = match self.direction {
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

    #[test]
    fn horizontal_fixed_size_children_compute_expected_starts() {
        let placement = BandSolution::from_sized_children(
            Orientation::Horizontal,
            &[
                input(0, 30.0, 80.0, 0.0, 0.0),
                input(1, 40.0, 90.0, 0.0, 0.0),
            ],
            TrackSpacing {
                outer_start: 5.0,
                outer_end: 7.0,
                min_gap: 10.0,
            },
            CrossAlign::default(),
        );

        assert_eq!(placement.children[0].main_start, 5.0);
        assert_eq!(placement.children[1].main_start, 45.0);
        assert_eq!(placement.main_extent, 92.0);
        assert_eq!(placement.cross_extent, Some(90.0));
    }

    #[test]
    fn vertical_fixed_size_children_compute_expected_starts() {
        let placement = BandSolution::from_sized_children(
            Orientation::Vertical,
            &[
                input(0, 12.0, 44.0, 0.0, 0.0),
                input(1, 18.0, 30.0, 0.0, 0.0),
            ],
            TrackSpacing {
                outer_start: 3.0,
                outer_end: 4.0,
                min_gap: 6.0,
            },
            CrossAlign::default(),
        );

        assert_eq!(placement.children[0].main_start, 3.0);
        assert_eq!(placement.children[1].main_start, 21.0);
        assert_eq!(placement.main_extent, 43.0);
        assert_eq!(placement.cross_extent, Some(44.0));
    }

    #[test]
    fn horizontal_center_alignment_offsets_smaller_children_on_cross_axis() {
        let placement = BandSolution::from_sized_children(
            Orientation::Horizontal,
            &[
                input(0, 30.0, 40.0, 0.0, 0.0),
                input(1, 30.0, 80.0, 0.0, 0.0),
            ],
            TrackSpacing {
                min_gap: 5.0,
                ..Default::default()
            },
            CrossAlign::Center,
        );

        assert_eq!(placement.cross_extent, Some(80.0));
        assert_eq!(placement.children[0].cross_start, 20.0);
        assert_eq!(placement.children[1].cross_start, 0.0);

        let result = placement.to_placement_solution([0.0, 10.0], Size::new(1.0, 2.0));
        assert_eq!(result.child(0).unwrap().origin, [0.0, 30.0]);
        assert_eq!(result.child(1).unwrap().origin, [35.0, 10.0]);
    }

    #[test]
    fn vertical_end_alignment_offsets_smaller_children_on_cross_axis() {
        let placement = BandSolution::from_sized_children(
            Orientation::Vertical,
            &[
                input(0, 20.0, 25.0, 0.0, 0.0),
                input(1, 20.0, 75.0, 0.0, 0.0),
            ],
            TrackSpacing {
                min_gap: 5.0,
                ..Default::default()
            },
            CrossAlign::End,
        );

        assert_eq!(placement.cross_extent, Some(75.0));
        assert_eq!(placement.children[0].cross_start, 50.0);
        assert_eq!(placement.children[1].cross_start, 0.0);

        let result = placement.to_placement_solution([7.0, 0.0], Size::new(1.0, 2.0));
        assert_eq!(result.child(0).unwrap().origin, [57.0, 0.0]);
        assert_eq!(result.child(1).unwrap().origin, [7.0, 25.0]);
    }

    #[test]
    fn minimum_gap_wins_when_larger_than_boundary_demand() {
        let placement = BandSolution::from_sized_children(
            Orientation::Horizontal,
            &[
                input(0, 20.0, 10.0, 0.0, 2.0),
                input(1, 20.0, 10.0, 3.0, 0.0),
            ],
            TrackSpacing {
                min_gap: 12.0,
                ..Default::default()
            },
            CrossAlign::default(),
        );

        assert_eq!(placement.children[1].main_start, 32.0);
        assert_eq!(placement.main_extent, 52.0);
    }

    #[test]
    fn boundary_demand_wins_when_larger_than_minimum_gap() {
        let placement = BandSolution::from_sized_children(
            Orientation::Horizontal,
            &[
                input(0, 20.0, 10.0, 0.0, 8.0),
                input(1, 20.0, 10.0, 7.0, 0.0),
            ],
            TrackSpacing {
                min_gap: 4.0,
                ..Default::default()
            },
            CrossAlign::default(),
        );

        assert_eq!(placement.children[1].main_start, 35.0);
        assert_eq!(placement.main_extent, 55.0);
    }

    #[test]
    fn padded_placeholder_slots_preserve_ragged_band_extent() {
        let spacing = TrackSpacing {
            min_gap: 10.0,
            ..Default::default()
        };
        let single = BandSolution::from_sized_children(
            Orientation::Horizontal,
            &[input(0, 100.0, 50.0, 0.0, 0.0)],
            spacing,
            CrossAlign::default(),
        );
        let padded = BandSolution::from_sized_children(
            Orientation::Horizontal,
            &[
                input(0, 100.0, 50.0, 0.0, 0.0),
                input(1, 100.0, 50.0, 0.0, 0.0),
                input(2, 100.0, 50.0, 0.0, 0.0),
            ],
            spacing,
            CrossAlign::default(),
        );

        assert_eq!(single.children[0].main_start, 0.0);
        assert_eq!(padded.children[0].main_start, 0.0);
        assert_eq!(single.main_extent, 100.0);
        assert_eq!(padded.main_extent, 320.0);
    }

    #[test]
    fn positioned_children_preserve_explicit_starts_and_sizes() {
        let placement = BandSolution::from_positioned_children(
            Orientation::Horizontal,
            vec![
                PlacedBandItem::with_cross_axis(3, 20.0, 40.0, 5.0, 45.0),
                PlacedBandItem::with_cross_axis(1, 80.0, 30.0, 12.0, 20.0),
            ],
            120.0,
            None,
        );

        assert_eq!(placement.children[0].id, 3);
        assert_eq!(placement.children[0].main_start, 20.0);
        assert_eq!(placement.children[1].main_size, 30.0);
        assert_eq!(placement.main_extent, 120.0);
        assert_eq!(placement.cross_extent, Some(50.0));
        assert_eq!(placement.children[1].cross_start, 12.0);
    }

    #[test]
    fn horizontal_conversion_maps_main_axis_to_x_origin() {
        let placement = BandSolution::from_positioned_children(
            Orientation::Horizontal,
            vec![PlacedBandItem::new(2, 30.0, 40.0)],
            100.0,
            Some(80.0),
        );

        let result = placement.to_placement_solution([5.0, 7.0], Size::new(1.0, 2.0));

        assert_eq!(result.content_size, Size::new(100.0, 80.0));
        assert_eq!(result.child(2).unwrap().origin, [35.0, 7.0]);
    }

    #[test]
    fn vertical_conversion_maps_main_axis_to_y_origin() {
        let placement = BandSolution::from_positioned_children(
            Orientation::Vertical,
            vec![PlacedBandItem::new(2, 30.0, 40.0)],
            100.0,
            Some(80.0),
        );

        let result = placement.to_placement_solution([5.0, 7.0], Size::new(1.0, 2.0));

        assert_eq!(result.content_size, Size::new(80.0, 100.0));
        assert_eq!(result.child(2).unwrap().origin, [5.0, 37.0]);
    }
}
