//! Child-frame placement utilities shared by container-style content.

use crate::layout::{EdgeSlabs, LayoutBounds, Size2D};

/// Coordinated edge targets granted to a child region by its parent container.
///
/// `inner` is the interior edge between the content rectangle and any outer
/// content. `total` is the full rendered edge envelope. The difference is
/// important for local outer-content anchoring: outer content should start
/// after the coordinated inner edge, while sibling spacing uses the coordinated
/// total edge.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct EdgeTargets {
    pub(crate) inner: EdgeSlabs,
    pub(crate) total: EdgeSlabs,
}

/// Render-space placement for one child frame relative to its parent content rectangle.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlacedRegion {
    pub(crate) child_index: usize,
    pub(crate) origin: [f32; 2],
    pub(crate) content_size_override: Option<Size2D>,
    pub(crate) edge_targets: Option<EdgeTargets>,
}

impl PlacedRegion {
    pub(crate) fn new(child_index: usize, origin: [f32; 2]) -> Self {
        Self {
            child_index,
            origin,
            content_size_override: None,
            edge_targets: None,
        }
    }

    pub(crate) fn with_content_size_override(
        child_index: usize,
        origin: [f32; 2],
        content_size_override: Size2D,
    ) -> Self {
        Self {
            child_index,
            origin,
            content_size_override: Some(content_size_override),
            edge_targets: None,
        }
    }

    pub(crate) fn with_edge_targets(mut self, targets: EdgeTargets) -> Self {
        self.edge_targets = Some(targets);
        self
    }
}

/// Placement result for child frames inside one parent content rectangle.
///
/// The result is intentionally independent of how placement was computed. A
/// facet band, concat container, coordinate-positioned container, or absolute
/// layout can all produce the same child-frame handoff for rendering and debug
/// projection.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlacementSolution {
    pub(crate) content_size: Size2D,
    pub(crate) placements: Vec<PlacedRegion>,
}

impl PlacementSolution {
    pub(crate) fn new(content_size: Size2D, placements: Vec<PlacedRegion>) -> Self {
        Self {
            content_size,
            placements,
        }
    }

    pub(crate) fn placements(&self) -> &[PlacedRegion] {
        &self.placements
    }

    pub(crate) fn child(&self, child_index: usize) -> Option<&PlacedRegion> {
        self.placements
            .iter()
            .find(|placement| placement.child_index == child_index)
    }
}

/// Project a child-local component bound into the parent frame's coordinate space.
pub(crate) fn project_child_rect(
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
    fn child_rect_projection_matches_render_group_transform() {
        let projected = project_child_rect(
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
    fn placement_solution_finds_child_by_index() {
        let result = PlacementSolution::new(
            Size2D::new(200.0, 120.0),
            vec![
                PlacedRegion::new(3, [10.0, 20.0]),
                PlacedRegion::new(1, [30.0, 40.0]),
            ],
        );

        assert_eq!(result.placements().len(), 2);
        assert_eq!(result.child(1).unwrap().origin, [30.0, 40.0]);
        assert!(result.child(2).is_none());
        assert_eq!(result.content_size, Size2D::new(200.0, 120.0));
    }
}
