//! Edge demand layering and the region placement handoff.

use crate::geometry::{Edges, Rect, Size};

/// Edge demand split into semantic layers while keeping one numeric total.
///
/// `inner` is interior chrome between the content rectangle and any outer
/// content; `outer` is content that stacks beyond the inner edge; `total` is
/// the full rendered envelope.
///
/// `new` LIFTS the total to `max(total, inner + outer, 0)`. This is an
/// intentional law, not input validation: when layered demands merge by
/// component-wise max, the lift makes a merged side's total equal
/// `max(inner) + max(outer)` — the space a region occupies once each layer
/// has been coordinated independently. Callers that need raw, unlifted
/// totals should compute envelopes with the geometric tree-envelope kind
/// instead of layering.
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

impl Edges<EdgeDemand> {
    /// Component-wise [`EdgeDemand::max_components`] on every side.
    ///
    /// This is the neutral merge law for per-side layered demand (for
    /// charts: guide overflow as `inner`, legend overflow as `outer`).
    pub fn max_components(self, other: Self) -> Self {
        Edges {
            top: self.top.max_components(other.top),
            right: self.right.max_components(other.right),
            bottom: self.bottom.max_components(other.bottom),
            left: self.left.max_components(other.left),
        }
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
///
/// `M` is caller-owned metadata carried through the handoff untouched (for
/// charts: retarget protocol state). The crate never reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedRegion<Id = usize, M = ()> {
    pub id: Id,
    pub origin: [f32; 2],
    pub meta: M,
}

impl<Id, M: Default> PlacedRegion<Id, M> {
    pub fn new(id: Id, origin: [f32; 2]) -> Self {
        Self {
            id,
            origin,
            meta: M::default(),
        }
    }
}

impl<Id, M> PlacedRegion<Id, M> {
    pub fn with_meta(id: Id, origin: [f32; 2], meta: M) -> Self {
        Self { id, origin, meta }
    }
}

/// Placement result for child regions inside one parent content rectangle.
///
/// The result is intentionally independent of how placement was computed. A
/// band, a grid, or an absolute layout can all produce the same handoff for
/// rendering and debug projection.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementSolution<Id = usize, M = ()> {
    pub content_size: Size,
    pub placements: Vec<PlacedRegion<Id, M>>,
}

impl<Id, M> PlacementSolution<Id, M> {
    pub fn new(content_size: Size, placements: Vec<PlacedRegion<Id, M>>) -> Self {
        Self {
            content_size,
            placements,
        }
    }

    pub fn placements(&self) -> &[PlacedRegion<Id, M>] {
        &self.placements
    }

    /// Lift every region's metadata into another metadata space.
    pub fn map_meta<N>(self, mut f: impl FnMut(M) -> N) -> PlacementSolution<Id, N> {
        PlacementSolution {
            content_size: self.content_size,
            placements: self
                .placements
                .into_iter()
                .map(|placement| PlacedRegion {
                    id: placement.id,
                    origin: placement.origin,
                    meta: f(placement.meta),
                })
                .collect(),
        }
    }
}

impl<Id: PartialEq, M> PlacementSolution<Id, M> {
    pub fn child(&self, id: Id) -> Option<&PlacedRegion<Id, M>> {
        self.placements.iter().find(|placement| placement.id == id)
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
    fn edge_demand_new_lifts_total_to_layer_sum() {
        // total 2.0 < inner 4.0 + outer 11.0: the constructor lifts it so
        // independently coordinated layers stay representable.
        assert_eq!(
            EdgeDemand::new(4.0, 11.0, 2.0),
            EdgeDemand {
                inner: 4.0,
                outer: 11.0,
                total: 15.0
            }
        );
    }

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
    fn placement_solution_finds_child_by_id() {
        let result: PlacementSolution = PlacementSolution::new(
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
