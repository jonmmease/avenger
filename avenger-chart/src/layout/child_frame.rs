//! Child-frame placement utilities shared by container-style content.

use crate::layout::LayoutBounds;

/// Render-space placement for one child frame relative to its parent content rectangle.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChildFrameRenderPlacement {
    pub(crate) child_index: usize,
    pub(crate) origin: [f32; 2],
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
}
