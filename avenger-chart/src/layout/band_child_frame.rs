//! One-dimensional child-frame band placement.
//!
//! This module is intentionally independent of facets. It places ordered child
//! frames along a horizontal or vertical band from child sizes, sibling boundary
//! demands, and spacing policy.

use crate::layout::{ChildFramePlacementResult, ChildFrameRenderPlacement, Size2D};

/// Flow direction for a one-dimensional band of child frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BandDirection {
    Horizontal,
    Vertical,
}

/// Cross-axis alignment for children inside a one-dimensional band.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(dead_code)] // Center/End are exercised by concat once that producer lands.
pub(crate) enum CrossAxisAlign {
    #[default]
    Start,
    Center,
    End,
}

/// Main-axis rendered demand outside one child frame boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct BoundaryDemand1D {
    pub(crate) before: f32,
    pub(crate) after: f32,
}

/// Sized child input for placement that owns its sibling gaps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BandChildFrameInput {
    pub(crate) child_index: usize,
    pub(crate) main_axis_size: f32,
    pub(crate) cross_axis_size: f32,
    pub(crate) boundary: BoundaryDemand1D,
}

/// Spacing policy for a band of child frames.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct BandSpacing {
    pub(crate) outer_start: f32,
    pub(crate) outer_end: f32,
    pub(crate) min_inner_gap: f32,
    pub(crate) cross_axis_align: CrossAxisAlign,
}

/// Positioned child frame in main/cross-axis coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BandPlacedChild {
    pub(crate) child_index: usize,
    pub(crate) main_axis_start: f32,
    pub(crate) main_axis_size: f32,
    pub(crate) cross_axis_start: f32,
    pub(crate) cross_axis_size: f32,
}

impl BandPlacedChild {
    pub(crate) fn new(child_index: usize, main_axis_start: f32, main_axis_size: f32) -> Self {
        Self {
            child_index,
            main_axis_start,
            main_axis_size,
            cross_axis_start: 0.0,
            cross_axis_size: 0.0,
        }
    }

    pub(crate) fn with_cross_axis(
        child_index: usize,
        main_axis_start: f32,
        main_axis_size: f32,
        cross_axis_start: f32,
        cross_axis_size: f32,
    ) -> Self {
        Self {
            child_index,
            main_axis_start,
            main_axis_size,
            cross_axis_start: cross_axis_start.max(0.0),
            cross_axis_size: cross_axis_size.max(0.0),
        }
    }
}

/// Placement result for a one-dimensional band of child frames.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BandChildFramePlacement {
    pub(crate) direction: BandDirection,
    pub(crate) children: Vec<BandPlacedChild>,
    pub(crate) main_axis_extent: f32,
    pub(crate) cross_axis_extent: Option<f32>,
}

impl BandChildFramePlacement {
    /// Build from child positions that have already been resolved by another
    /// system, such as a configured band scale.
    pub(crate) fn from_positioned_children(
        direction: BandDirection,
        children: Vec<BandPlacedChild>,
        main_axis_extent: f32,
        cross_axis_extent: Option<f32>,
    ) -> Self {
        let cross_axis_extent = cross_axis_extent.or_else(|| {
            if children.is_empty() {
                None
            } else {
                Some(
                    children
                        .iter()
                        .map(|child| child.cross_axis_start + child.cross_axis_size)
                        .fold(0.0f32, f32::max),
                )
            }
        });

        Self {
            direction,
            children,
            main_axis_extent,
            cross_axis_extent,
        }
    }

    /// Build from child sizes, sibling boundary demands, and spacing policy.
    pub(crate) fn from_sized_children(
        direction: BandDirection,
        children: &[BandChildFrameInput],
        spacing: BandSpacing,
    ) -> Self {
        let mut placed_children = Vec::with_capacity(children.len());
        let mut cursor = spacing.outer_start.max(0.0);
        let cross_axis_extent = children
            .iter()
            .map(|child| child.cross_axis_size.max(0.0))
            .fold(0.0f32, f32::max);
        let min_inner_gap = spacing.min_inner_gap.max(0.0);

        for (idx, child) in children.iter().enumerate() {
            let main_axis_size = child.main_axis_size.max(0.0);
            let child_cross_axis_size = child.cross_axis_size.max(0.0);
            let cross_axis_start = spacing
                .cross_axis_align
                .offset(cross_axis_extent, child_cross_axis_size);
            placed_children.push(BandPlacedChild::with_cross_axis(
                child.child_index,
                cursor,
                main_axis_size,
                cross_axis_start,
                child_cross_axis_size,
            ));

            cursor += main_axis_size;
            if let Some(next) = children.get(idx + 1) {
                let boundary_gap = child.boundary.after.max(0.0) + next.boundary.before.max(0.0);
                cursor += min_inner_gap.max(boundary_gap);
            }
        }

        Self {
            direction,
            children: placed_children,
            main_axis_extent: (cursor + spacing.outer_end.max(0.0)).max(0.0),
            cross_axis_extent: Some(cross_axis_extent),
        }
    }

    /// Convert main/cross-axis child placement into render-space origins
    /// relative to the parent content rectangle.
    pub(crate) fn to_child_frame_placement_result(
        &self,
        origin_offset: [f32; 2],
        fallback_content_size: Size2D,
    ) -> ChildFramePlacementResult {
        let render_placements = self
            .children
            .iter()
            .map(|child| {
                let origin = match self.direction {
                    BandDirection::Horizontal => [
                        child.main_axis_start + origin_offset[0],
                        child.cross_axis_start + origin_offset[1],
                    ],
                    BandDirection::Vertical => [
                        child.cross_axis_start + origin_offset[0],
                        child.main_axis_start + origin_offset[1],
                    ],
                };
                ChildFrameRenderPlacement::new(child.child_index, origin)
            })
            .collect();

        let content_size = match self.direction {
            BandDirection::Horizontal => Size2D::new(
                self.main_axis_extent,
                self.cross_axis_extent
                    .unwrap_or(fallback_content_size.height),
            ),
            BandDirection::Vertical => Size2D::new(
                self.cross_axis_extent
                    .unwrap_or(fallback_content_size.width),
                self.main_axis_extent,
            ),
        };

        ChildFramePlacementResult::new(content_size, render_placements)
    }
}

impl CrossAxisAlign {
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

    fn input(
        child_index: usize,
        main_axis_size: f32,
        cross_axis_size: f32,
        before: f32,
        after: f32,
    ) -> BandChildFrameInput {
        BandChildFrameInput {
            child_index,
            main_axis_size,
            cross_axis_size,
            boundary: BoundaryDemand1D { before, after },
        }
    }

    #[test]
    fn horizontal_fixed_size_children_compute_expected_starts() {
        let placement = BandChildFramePlacement::from_sized_children(
            BandDirection::Horizontal,
            &[
                input(0, 30.0, 80.0, 0.0, 0.0),
                input(1, 40.0, 90.0, 0.0, 0.0),
            ],
            BandSpacing {
                outer_start: 5.0,
                outer_end: 7.0,
                min_inner_gap: 10.0,
                ..Default::default()
            },
        );

        assert_eq!(placement.children[0].main_axis_start, 5.0);
        assert_eq!(placement.children[1].main_axis_start, 45.0);
        assert_eq!(placement.main_axis_extent, 92.0);
        assert_eq!(placement.cross_axis_extent, Some(90.0));
    }

    #[test]
    fn vertical_fixed_size_children_compute_expected_starts() {
        let placement = BandChildFramePlacement::from_sized_children(
            BandDirection::Vertical,
            &[
                input(0, 12.0, 44.0, 0.0, 0.0),
                input(1, 18.0, 30.0, 0.0, 0.0),
            ],
            BandSpacing {
                outer_start: 3.0,
                outer_end: 4.0,
                min_inner_gap: 6.0,
                ..Default::default()
            },
        );

        assert_eq!(placement.children[0].main_axis_start, 3.0);
        assert_eq!(placement.children[1].main_axis_start, 21.0);
        assert_eq!(placement.main_axis_extent, 43.0);
        assert_eq!(placement.cross_axis_extent, Some(44.0));
    }

    #[test]
    fn horizontal_center_alignment_offsets_smaller_children_on_cross_axis() {
        let placement = BandChildFramePlacement::from_sized_children(
            BandDirection::Horizontal,
            &[
                input(0, 30.0, 40.0, 0.0, 0.0),
                input(1, 30.0, 80.0, 0.0, 0.0),
            ],
            BandSpacing {
                min_inner_gap: 5.0,
                cross_axis_align: CrossAxisAlign::Center,
                ..Default::default()
            },
        );

        assert_eq!(placement.cross_axis_extent, Some(80.0));
        assert_eq!(placement.children[0].cross_axis_start, 20.0);
        assert_eq!(placement.children[1].cross_axis_start, 0.0);

        let result = placement.to_child_frame_placement_result([0.0, 10.0], Size2D::new(1.0, 2.0));
        assert_eq!(result.child(0).unwrap().origin, [0.0, 30.0]);
        assert_eq!(result.child(1).unwrap().origin, [35.0, 10.0]);
    }

    #[test]
    fn vertical_end_alignment_offsets_smaller_children_on_cross_axis() {
        let placement = BandChildFramePlacement::from_sized_children(
            BandDirection::Vertical,
            &[
                input(0, 20.0, 25.0, 0.0, 0.0),
                input(1, 20.0, 75.0, 0.0, 0.0),
            ],
            BandSpacing {
                min_inner_gap: 5.0,
                cross_axis_align: CrossAxisAlign::End,
                ..Default::default()
            },
        );

        assert_eq!(placement.cross_axis_extent, Some(75.0));
        assert_eq!(placement.children[0].cross_axis_start, 50.0);
        assert_eq!(placement.children[1].cross_axis_start, 0.0);

        let result = placement.to_child_frame_placement_result([7.0, 0.0], Size2D::new(1.0, 2.0));
        assert_eq!(result.child(0).unwrap().origin, [57.0, 0.0]);
        assert_eq!(result.child(1).unwrap().origin, [7.0, 25.0]);
    }

    #[test]
    fn minimum_gap_wins_when_larger_than_boundary_demand() {
        let placement = BandChildFramePlacement::from_sized_children(
            BandDirection::Horizontal,
            &[
                input(0, 20.0, 10.0, 0.0, 2.0),
                input(1, 20.0, 10.0, 3.0, 0.0),
            ],
            BandSpacing {
                min_inner_gap: 12.0,
                ..Default::default()
            },
        );

        assert_eq!(placement.children[1].main_axis_start, 32.0);
        assert_eq!(placement.main_axis_extent, 52.0);
    }

    #[test]
    fn boundary_demand_wins_when_larger_than_minimum_gap() {
        let placement = BandChildFramePlacement::from_sized_children(
            BandDirection::Horizontal,
            &[
                input(0, 20.0, 10.0, 0.0, 8.0),
                input(1, 20.0, 10.0, 7.0, 0.0),
            ],
            BandSpacing {
                min_inner_gap: 4.0,
                ..Default::default()
            },
        );

        assert_eq!(placement.children[1].main_axis_start, 35.0);
        assert_eq!(placement.main_axis_extent, 55.0);
    }

    #[test]
    fn positioned_children_preserve_explicit_starts_and_sizes() {
        let placement = BandChildFramePlacement::from_positioned_children(
            BandDirection::Horizontal,
            vec![
                BandPlacedChild::with_cross_axis(3, 20.0, 40.0, 5.0, 45.0),
                BandPlacedChild::with_cross_axis(1, 80.0, 30.0, 12.0, 20.0),
            ],
            120.0,
            None,
        );

        assert_eq!(placement.children[0].child_index, 3);
        assert_eq!(placement.children[0].main_axis_start, 20.0);
        assert_eq!(placement.children[1].main_axis_size, 30.0);
        assert_eq!(placement.main_axis_extent, 120.0);
        assert_eq!(placement.cross_axis_extent, Some(50.0));
        assert_eq!(placement.children[1].cross_axis_start, 12.0);
    }

    #[test]
    fn horizontal_conversion_maps_main_axis_to_x_origin() {
        let placement = BandChildFramePlacement::from_positioned_children(
            BandDirection::Horizontal,
            vec![BandPlacedChild::new(2, 30.0, 40.0)],
            100.0,
            Some(80.0),
        );

        let result = placement.to_child_frame_placement_result([5.0, 7.0], Size2D::new(1.0, 2.0));

        assert_eq!(result.content_size, Size2D::new(100.0, 80.0));
        assert_eq!(result.child(2).unwrap().origin, [35.0, 7.0]);
    }

    #[test]
    fn vertical_conversion_maps_main_axis_to_y_origin() {
        let placement = BandChildFramePlacement::from_positioned_children(
            BandDirection::Vertical,
            vec![BandPlacedChild::new(2, 30.0, 40.0)],
            100.0,
            Some(80.0),
        );

        let result = placement.to_child_frame_placement_result([5.0, 7.0], Size2D::new(1.0, 2.0));

        assert_eq!(result.content_size, Size2D::new(80.0, 100.0));
        assert_eq!(result.child(2).unwrap().origin, [5.0, 37.0]);
    }
}
