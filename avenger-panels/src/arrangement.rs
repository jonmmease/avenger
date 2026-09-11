use crate::{GridShape, GridSlot, GroupId, NodeId, PanelError, PanelId, PanelTree};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroUsize,
};

/// Whether a panel supplies guides in this physical arrangement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelDisplay {
    /// Display the panel, including when its data set is empty.
    #[default]
    Shown,
    /// Preserve its slot without displaying panel contents or guides.
    Hole,
}

/// Physical placement of the direct children of one logical group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupArrangement {
    /// One horizontal row in tree order.
    Row,
    /// One vertical column in tree order.
    Column,
    /// Row-major wrapping with this many columns.
    Wrap(NonZeroUsize),
    /// Explicit slots, including spans and unused cells.
    Grid {
        /// Number of physical rows and columns.
        shape: GridShape,
        /// One slot for each direct child.
        slots: Vec<(NodeId, GridSlot)>,
    },
}

/// Declarations for every group, plus explicit panel holes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArrangementSpec {
    groups: BTreeMap<GroupId, GroupArrangement>,
    display: BTreeMap<PanelId, PanelDisplay>,
}
impl ArrangementSpec {
    /// Start with no group declarations and every panel shown.
    pub fn new() -> Self {
        Self::default()
    }
    /// Set or replace one group's physical arrangement.
    pub fn group(mut self, group: impl Into<GroupId>, arrangement: GroupArrangement) -> Self {
        self.groups.insert(group.into(), arrangement);
        self
    }
    /// Set or replace a panel's display state.
    pub fn display(mut self, panel: impl Into<PanelId>, display: PanelDisplay) -> Self {
        self.display.insert(panel.into(), display);
        self
    }
}

/// One validated physical grid with children in logical tree order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelGrid {
    shape: GridShape,
    slots: Vec<(NodeId, GridSlot)>,
}
impl PanelGrid {
    /// Physical track counts.
    pub fn shape(&self) -> GridShape {
        self.shape
    }
    /// Child identities and their slots in tree order.
    pub fn slots(&self) -> &[(NodeId, GridSlot)] {
        &self.slots
    }
}

/// A physical slot assignment that retains the original logical tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelArrangement {
    pub(crate) tree: PanelTree,
    grids: BTreeMap<GroupId, PanelGrid>,
    pub(crate) display: BTreeMap<PanelId, PanelDisplay>,
}
impl PanelArrangement {
    /// Logical hierarchy used to build this arrangement.
    pub fn tree(&self) -> &PanelTree {
        &self.tree
    }
    /// One group's resolved grid, or `None` for an unknown group.
    pub fn grid(&self, group: &GroupId) -> Option<&PanelGrid> {
        self.grids.get(group)
    }
    /// Display state, or `None` for an unknown panel.
    pub fn display(&self, panel: &PanelId) -> Option<PanelDisplay> {
        self.display.get(panel).copied()
    }
}

impl PanelTree {
    /// Resolve physical slots. Missing groups, invalid slots, and unknown IDs fail.
    pub fn arrange(&self, spec: &ArrangementSpec) -> Result<PanelArrangement, PanelError> {
        for id in spec.groups.keys() {
            if self.children(id).is_none() {
                return Err(PanelError::UnknownNode(NodeId::Group(id.clone())));
            }
        }
        for id in spec.display.keys() {
            if !self.contains(&NodeId::Panel(id.clone())) {
                return Err(PanelError::UnknownNode(NodeId::Panel(id.clone())));
            }
        }
        let mut grids = BTreeMap::new();
        for node in self.nodes() {
            let NodeId::Group(id) = node else { continue };
            let fail = |reason: &str| PanelError::InvalidArrangement {
                group: id.clone(),
                reason: reason.into(),
            };
            let children = self.children(id).expect("validated group");
            let declaration = spec
                .groups
                .get(id)
                .ok_or_else(|| fail("missing group declaration"))?;
            let (shape, slots) = match declaration {
                GroupArrangement::Grid { shape, slots } => (*shape, slots.clone()),
                _ => {
                    let count = children.len();
                    let columns = match declaration {
                        GroupArrangement::Row => count.max(1),
                        GroupArrangement::Column => 1,
                        GroupArrangement::Wrap(columns) => columns.get(),
                        GroupArrangement::Grid { .. } => unreachable!(),
                    };
                    let shape = if count == 0 {
                        GridShape {
                            rows: 0,
                            columns: 0,
                        }
                    } else {
                        GridShape {
                            rows: count.div_ceil(columns),
                            columns,
                        }
                    };
                    let slots = children
                        .iter()
                        .enumerate()
                        .map(|(i, id)| {
                            (
                                id.clone(),
                                GridSlot {
                                    row: i / columns,
                                    column: i % columns,
                                    row_span: 1,
                                    column_span: 1,
                                },
                            )
                        })
                        .collect();
                    (shape, slots)
                }
            };
            let mut seen = BTreeSet::new();
            for (child, slot) in &slots {
                if !children.contains(child) {
                    return Err(fail("slot refers to a node that is not a direct child"));
                }
                if !seen.insert(child) {
                    return Err(fail("child occupies more than one slot"));
                }
                if slot.row_span == 0
                    || slot.column_span == 0
                    || slot
                        .row
                        .checked_add(slot.row_span)
                        .is_none_or(|end| end > shape.rows)
                    || slot
                        .column
                        .checked_add(slot.column_span)
                        .is_none_or(|end| end > shape.columns)
                {
                    return Err(fail("slot has a zero span or exceeds the grid"));
                }
            }
            if seen.len() != children.len() {
                return Err(fail("missing child slot"));
            }
            for (i, (_, a)) in slots.iter().enumerate() {
                for (_, b) in &slots[i + 1..] {
                    if a.row < b.row_end()
                        && b.row < a.row_end()
                        && a.column < b.column_end()
                        && b.column < a.column_end()
                    {
                        return Err(fail("child slots overlap"));
                    }
                }
            }
            let by_id: BTreeMap<_, _> = slots.into_iter().collect();
            grids.insert(
                id.clone(),
                PanelGrid {
                    shape,
                    slots: children.iter().map(|c| (c.clone(), by_id[c])).collect(),
                },
            );
        }
        let display = self
            .panels()
            .map(|p| (p.clone(), spec.display.get(p).copied().unwrap_or_default()))
            .collect();
        Ok(PanelArrangement {
            tree: self.clone(),
            grids,
            display,
        })
    }
}
