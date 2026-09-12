use crate::{NodeId, PanelArrangement, PanelDisplay, PanelError, PanelId, PanelTree, Rect};
use avenger_layout::LayoutSolution;
use std::collections::BTreeMap;

/// Validated plot and group content rectangles in root-relative logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelFrames {
    pub(crate) tree: PanelTree,
    rects: BTreeMap<NodeId, Rect>,
    display: BTreeMap<PanelId, PanelDisplay>,
}
impl PanelFrames {
    /// Validate one rectangle per logical node and the panel IDs displayed as holes.
    ///
    /// Coordinates and endpoints must be finite, and sizes must be nonnegative.
    /// Missing, duplicate, and unknown identities are errors.
    pub fn new(
        tree: &PanelTree,
        frames: impl IntoIterator<Item = (NodeId, Rect)>,
        holes: impl IntoIterator<Item = PanelId>,
    ) -> Result<Self, PanelError> {
        let mut rects = BTreeMap::new();
        for (id, rect) in frames {
            if !tree.contains(&id) {
                return Err(PanelError::UnknownNode(id));
            }
            if ![
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                rect.x + rect.width,
                rect.y + rect.height,
            ]
            .iter()
            .all(|n| n.is_finite())
                || rect.width < 0.0
                || rect.height < 0.0
            {
                return Err(PanelError::InvalidFrame { node: id, rect });
            }
            if rects.insert(id.clone(), rect).is_some() {
                return Err(PanelError::DuplicateNode(id));
            }
        }
        for id in tree.nodes() {
            if !rects.contains_key(id) {
                return Err(PanelError::MissingFrame(id.clone()));
            }
        }
        let mut display: BTreeMap<_, _> = tree
            .panels()
            .map(|p| (p.clone(), PanelDisplay::Shown))
            .collect();
        for id in holes {
            let state = display
                .get_mut(&id)
                .ok_or_else(|| PanelError::UnknownNode(NodeId::Panel(id.clone())))?;
            if *state == PanelDisplay::Hole {
                return Err(PanelError::DuplicateParticipant(id));
            }
            *state = PanelDisplay::Hole;
        }
        Ok(Self {
            tree: tree.clone(),
            rects,
            display,
        })
    }
    /// Read `Region::content` for registered nodes, retaining arrangement display states.
    ///
    /// Register an invisible layout node for each retained hole. Untagged wrapper
    /// regions are ignored. Duplicate node registrations are errors.
    pub fn from_layout(
        arrangement: &PanelArrangement,
        solution: &LayoutSolution<NodeId>,
    ) -> Result<Self, PanelError> {
        Self::new(
            &arrangement.tree,
            solution
                .regions()
                .filter_map(|r| r.id.as_ref().map(|id| (id.clone(), r.content))),
            arrangement
                .display
                .iter()
                .filter(|(_, d)| **d == PanelDisplay::Hole)
                .map(|(p, _)| p.clone()),
        )
    }
    /// Content rectangle, or `None` for an unknown identity.
    pub fn rect(&self, node: &NodeId) -> Option<Rect> {
        self.rects.get(node).copied()
    }
    /// Panel display state, or `None` for an unknown panel.
    pub fn display(&self, panel: &PanelId) -> Option<PanelDisplay> {
        self.display.get(panel).copied()
    }
    pub(crate) fn visible(&self, panel: &PanelId) -> bool {
        self.display(panel) == Some(PanelDisplay::Shown)
            && self
                .rect(&NodeId::Panel(panel.clone()))
                .is_some_and(|r| r.width > 0.0 && r.height > 0.0)
    }
}
