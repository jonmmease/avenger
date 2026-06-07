//! Child-frame placement utilities shared by container-style content.

use crate::layout::{LayoutBounds, Size2D};

/// Render-space placement for one child frame relative to its parent content rectangle.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChildFrameRenderPlacement {
    pub(crate) child_index: usize,
    pub(crate) origin: [f32; 2],
    pub(crate) plot_area_size: Option<Size2D>,
}

impl ChildFrameRenderPlacement {
    pub(crate) fn new(child_index: usize, origin: [f32; 2]) -> Self {
        Self {
            child_index,
            origin,
            plot_area_size: None,
        }
    }

    pub(crate) fn with_plot_area_size(
        child_index: usize,
        origin: [f32; 2],
        plot_area_size: Size2D,
    ) -> Self {
        Self {
            child_index,
            origin,
            plot_area_size: Some(plot_area_size),
        }
    }
}

/// Placement result for child frames inside one parent content rectangle.
///
/// The result is intentionally independent of how placement was computed. A
/// facet band, concat container, coordinate-positioned container, or absolute
/// layout can all produce the same child-frame handoff for rendering and debug
/// projection.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChildFramePlacementResult {
    pub(crate) content_size: Size2D,
    pub(crate) render_placements: Vec<ChildFrameRenderPlacement>,
}

impl ChildFramePlacementResult {
    pub(crate) fn new(
        content_size: Size2D,
        render_placements: Vec<ChildFrameRenderPlacement>,
    ) -> Self {
        Self {
            content_size,
            render_placements,
        }
    }

    pub(crate) fn render_placements(&self) -> &[ChildFrameRenderPlacement] {
        &self.render_placements
    }

    pub(crate) fn child(&self, child_index: usize) -> Option<&ChildFrameRenderPlacement> {
        self.render_placements
            .iter()
            .find(|placement| placement.child_index == child_index)
    }
}

/// Project a child-local component bound into the parent frame's coordinate space.
pub(crate) fn project_child_frame_bounds(
    parent_content_origin: [f32; 2],
    child_render_origin: [f32; 2],
    child_plot_bounds: LayoutBounds,
    child_bounds: LayoutBounds,
) -> LayoutBounds {
    LayoutBounds {
        x: parent_content_origin[0] + child_render_origin[0] + child_bounds.x - child_plot_bounds.x,
        y: parent_content_origin[1] + child_render_origin[1] + child_bounds.y - child_plot_bounds.y,
        width: child_bounds.width,
        height: child_bounds.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_frame_bounds_projection_matches_render_group_transform() {
        let projected = project_child_frame_bounds(
            [10.0, 85.0],
            [20.0, 48.0],
            LayoutBounds {
                x: 5.0,
                y: 89.0,
                width: 100.0,
                height: 80.0,
            },
            LayoutBounds {
                x: 7.0,
                y: 0.0,
                width: 30.0,
                height: 20.0,
            },
        );

        assert_eq!(
            projected,
            LayoutBounds {
                x: 32.0,
                y: 44.0,
                width: 30.0,
                height: 20.0
            }
        );
    }

    #[test]
    fn child_frame_placement_result_finds_child_by_index() {
        let result = ChildFramePlacementResult::new(
            Size2D::new(200.0, 120.0),
            vec![
                ChildFrameRenderPlacement::new(3, [10.0, 20.0]),
                ChildFrameRenderPlacement::new(1, [30.0, 40.0]),
            ],
        );

        assert_eq!(result.render_placements().len(), 2);
        assert_eq!(result.child(1).unwrap().origin, [30.0, 40.0]);
        assert!(result.child(2).is_none());
        assert_eq!(result.content_size, Size2D::new(200.0, 120.0));
    }
}
