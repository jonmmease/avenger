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
}

/// Positioned child frame in main/cross-axis coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BandPlacedChild {
    pub(crate) child_index: usize,
    pub(crate) main_axis_start: f32,
    pub(crate) main_axis_size: f32,
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
        let mut cross_axis_extent = 0.0f32;
        let min_inner_gap = spacing.min_inner_gap.max(0.0);

        for (idx, child) in children.iter().enumerate() {
            let main_axis_size = child.main_axis_size.max(0.0);
            cross_axis_extent = cross_axis_extent.max(child.cross_axis_size.max(0.0));
            placed_children.push(BandPlacedChild {
                child_index: child.child_index,
                main_axis_start: cursor,
                main_axis_size,
            });

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
                    BandDirection::Horizontal => {
                        [child.main_axis_start + origin_offset[0], origin_offset[1]]
                    }
                    BandDirection::Vertical => {
                        [origin_offset[0], child.main_axis_start + origin_offset[1]]
                    }
                };
                ChildFrameRenderPlacement {
                    child_index: child.child_index,
                    origin,
                }
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
            },
        );

        assert_eq!(placement.children[0].main_axis_start, 3.0);
        assert_eq!(placement.children[1].main_axis_start, 21.0);
        assert_eq!(placement.main_axis_extent, 43.0);
        assert_eq!(placement.cross_axis_extent, Some(44.0));
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
                BandPlacedChild {
                    child_index: 3,
                    main_axis_start: 20.0,
                    main_axis_size: 40.0,
                },
                BandPlacedChild {
                    child_index: 1,
                    main_axis_start: 80.0,
                    main_axis_size: 30.0,
                },
            ],
            120.0,
            Some(50.0),
        );

        assert_eq!(placement.children[0].child_index, 3);
        assert_eq!(placement.children[0].main_axis_start, 20.0);
        assert_eq!(placement.children[1].main_axis_size, 30.0);
        assert_eq!(placement.main_axis_extent, 120.0);
        assert_eq!(placement.cross_axis_extent, Some(50.0));
    }

    #[test]
    fn horizontal_conversion_maps_main_axis_to_x_origin() {
        let placement = BandChildFramePlacement::from_positioned_children(
            BandDirection::Horizontal,
            vec![BandPlacedChild {
                child_index: 2,
                main_axis_start: 30.0,
                main_axis_size: 40.0,
            }],
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
            vec![BandPlacedChild {
                child_index: 2,
                main_axis_start: 30.0,
                main_axis_size: 40.0,
            }],
            100.0,
            Some(80.0),
        );

        let result = placement.to_child_frame_placement_result([5.0, 7.0], Size2D::new(1.0, 2.0));

        assert_eq!(result.content_size, Size2D::new(80.0, 100.0));
        assert_eq!(result.child(2).unwrap().origin, [5.0, 37.0]);
    }
}
