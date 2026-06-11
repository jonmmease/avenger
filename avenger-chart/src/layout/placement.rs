//! Chart-owned placement handoff data: positioned children relative to a
//! parent content rectangle, with chart metadata, plus the child-rect
//! projection used by rendering.

use avenger_layout::{Edges, Rect, Size};

/// Coordinated edge targets granted to a child region by its parent.
///
/// `inner` is the interior edge between the content rectangle and any outer
/// content; `total` is the full rendered edge envelope. The difference
/// matters for local outer-content anchoring: outer content starts after the
/// coordinated inner edge, while sibling spacing uses the coordinated total.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeTargets {
    pub inner: Edges<f32>,
    pub total: Edges<f32>,
}

/// Placement for one child region relative to its parent content rectangle.
/// `M` is chart metadata (the concat retarget protocol state).
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
/// `child_reference` is the child-local rectangle that `child_origin`
/// points at (the child's plot area); `child_rect` is expressed in the same
/// child-local space.
pub(crate) fn project_rect(
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
