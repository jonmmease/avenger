//! Edge demand layering and the region placement handoff.

use crate::geometry::{Edges, Rect, Size};

/// Edge demand split into semantic layers while keeping one numeric total.
///
/// `inner` is interior chrome between the content rectangle and any outer
/// content; `outer` is content that stacks beyond the inner edge; `total` is
/// the full rendered envelope. The constructor maintains the invariant
/// `total >= max(inner + outer, 0)` by clamping.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdgeDemand {
    pub inner: f32,
    pub outer: f32,
    pub total: f32,
}

impl EdgeDemand {
    pub fn new(inner: f32, outer: f32, total: f32) -> Self {
        let inner = inner.max(0.0);
        let outer = outer.max(0.0);
        let total = total.max(inner + outer).max(0.0);
        Self {
            inner,
            outer,
            total,
        }
    }

    pub fn total(total: f32) -> Self {
        Self::new(0.0, 0.0, total)
    }

    pub fn max_components(self, other: Self) -> Self {
        Self::new(
            self.inner.max(other.inner),
            self.outer.max(other.outer),
            self.total.max(other.total),
        )
    }
}

/// Coordinated edge targets granted to a child region by its parent.
///
/// `inner` is the interior edge between the content rectangle and any outer
/// content. `total` is the full rendered edge envelope. The difference is
/// important for local outer-content anchoring: outer content should start
/// after the coordinated inner edge, while sibling spacing uses the
/// coordinated total edge.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeTargets {
    pub inner: Edges<f32>,
    pub total: Edges<f32>,
}

/// Placement for one child region relative to its parent content rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedRegion<Id = usize> {
    pub child_index: Id,
    pub origin: [f32; 2],
    pub content_size_override: Option<Size>,
    pub edge_targets: Option<EdgeTargets>,
}

impl<Id> PlacedRegion<Id> {
    pub fn new(child_index: Id, origin: [f32; 2]) -> Self {
        Self {
            child_index,
            origin,
            content_size_override: None,
            edge_targets: None,
        }
    }

    pub fn with_content_size_override(
        child_index: Id,
        origin: [f32; 2],
        content_size_override: Size,
    ) -> Self {
        Self {
            child_index,
            origin,
            content_size_override: Some(content_size_override),
            edge_targets: None,
        }
    }

    pub fn with_edge_targets(mut self, targets: EdgeTargets) -> Self {
        self.edge_targets = Some(targets);
        self
    }
}

/// Placement result for child regions inside one parent content rectangle.
///
/// The result is intentionally independent of how placement was computed. A
/// band, a grid, or an absolute layout can all produce the same handoff for
/// rendering and debug projection.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementSolution<Id = usize> {
    pub content_size: Size,
    pub placements: Vec<PlacedRegion<Id>>,
}

impl<Id> PlacementSolution<Id> {
    pub fn new(content_size: Size, placements: Vec<PlacedRegion<Id>>) -> Self {
        Self {
            content_size,
            placements,
        }
    }

    pub fn placements(&self) -> &[PlacedRegion<Id>] {
        &self.placements
    }
}

impl<Id: PartialEq> PlacementSolution<Id> {
    pub fn child(&self, child_index: Id) -> Option<&PlacedRegion<Id>> {
        self.placements
            .iter()
            .find(|placement| placement.child_index == child_index)
    }
}

/// Project a child-local rectangle into the parent's coordinate space.
///
/// `child_reference` is the child-local rectangle that `child_origin` points
/// at (for charts: the child's plot area); `child_rect` is expressed in the
/// same child-local space.
pub fn project_rect(
    parent_content_origin: [f32; 2],
    child_origin: [f32; 2],
    child_reference: Rect,
    child_rect: Rect,
) -> Rect {
    Rect {
        x: parent_content_origin[0] + child_origin[0] + child_rect.x - child_reference.x,
        y: parent_content_origin[1] + child_origin[1] + child_rect.y - child_reference.y,
        width: child_rect.width,
        height: child_rect.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_demand_merges_structured_components() {
        let left = EdgeDemand::new(4.0, 11.0, 2.0);
        assert_eq!(
            left,
            EdgeDemand {
                inner: 4.0,
                outer: 11.0,
                total: 15.0
            }
        );

        let right = EdgeDemand::new(9.0, 3.0, 22.0);
        assert_eq!(
            left.max_components(right),
            EdgeDemand {
                inner: 9.0,
                outer: 11.0,
                total: 22.0
            }
        );
    }

    #[test]
    fn rect_projection_matches_render_group_transform() {
        let projected = project_rect(
            [10.0, 85.0],
            [20.0, 48.0],
            Rect {
                x: 5.0,
                y: 89.0,
                width: 100.0,
                height: 80.0,
            },
            Rect {
                x: 7.0,
                y: 0.0,
                width: 30.0,
                height: 20.0,
            },
        );

        assert_eq!(
            projected,
            Rect {
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
            Size::new(200.0, 120.0),
            vec![
                PlacedRegion::new(3, [10.0, 20.0]),
                PlacedRegion::new(1, [30.0, 40.0]),
            ],
        );

        assert_eq!(result.placements().len(), 2);
        assert_eq!(result.child(1).unwrap().origin, [30.0, 40.0]);
        assert!(result.child(2).is_none());
        assert_eq!(result.content_size, Size::new(200.0, 120.0));
    }
}
