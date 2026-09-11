use crate::{GroupId, NodeId, PanelError, PanelId};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroUsize,
};

/// Logical ancestor at which participating panels meet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Each panel is independent.
    Panel,
    /// Ascend exactly this many logical parent links.
    Ancestor(NonZeroUsize),
    /// Share at the root group.
    Root,
    /// Share at this explicit ancestor group.
    Group(GroupId),
}
impl Scope {
    /// Construct a positive ancestor depth. A depth beyond the root fails on resolution.
    pub fn ancestor(levels: usize) -> Result<Self, PanelError> {
        NonZeroUsize::new(levels)
            .map(Self::Ancestor)
            .ok_or(PanelError::InvalidDepth {
                panel: None,
                levels,
            })
    }
}

/// A node declaration. Physical wrappers do not belong in this tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelNode {
    id: NodeId,
    children: Vec<PanelNode>,
}
impl PanelNode {
    /// Declare a plot area with a stable ID.
    pub fn panel(id: impl Into<PanelId>) -> Self {
        Self {
            id: NodeId::Panel(id.into()),
            children: Vec::new(),
        }
    }
    /// Declare an ordered logical group.
    pub fn group(id: impl Into<GroupId>, children: impl IntoIterator<Item = Self>) -> Self {
        Self {
            id: NodeId::Group(id.into()),
            children: children.into_iter().collect(),
        }
    }
}

/// An immutable validated hierarchy, independent of arrangement and data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelTree {
    root: GroupId,
    nodes: Vec<NodeId>,
    panels: Vec<PanelId>,
    parents: BTreeMap<NodeId, GroupId>,
    children: BTreeMap<GroupId, Vec<NodeId>>,
}
impl PanelTree {
    /// Validate an ordered hierarchy. Duplicate IDs within a namespace are errors.
    pub fn new(
        root: GroupId,
        children: impl IntoIterator<Item = PanelNode>,
    ) -> Result<Self, PanelError> {
        let mut tree = Self {
            root: root.clone(),
            nodes: Vec::new(),
            panels: Vec::new(),
            parents: BTreeMap::new(),
            children: BTreeMap::new(),
        };
        let mut seen = BTreeSet::new();
        tree.visit(PanelNode::group(root, children), None, &mut seen)?;
        Ok(tree)
    }
    fn visit(
        &mut self,
        node: PanelNode,
        parent: Option<GroupId>,
        seen: &mut BTreeSet<NodeId>,
    ) -> Result<(), PanelError> {
        if !seen.insert(node.id.clone()) {
            return Err(PanelError::DuplicateNode(node.id));
        }
        self.nodes.push(node.id.clone());
        if let Some(parent) = parent {
            self.parents.insert(node.id.clone(), parent);
        }
        match node.id {
            NodeId::Panel(id) => self.panels.push(id),
            NodeId::Group(id) => {
                self.children.insert(
                    id.clone(),
                    node.children.iter().map(|c| c.id.clone()).collect(),
                );
                for child in node.children {
                    self.visit(child, Some(id.clone()), seen)?;
                }
            }
        }
        Ok(())
    }
    /// Root identity.
    pub fn root(&self) -> &GroupId {
        &self.root
    }
    /// Panel identities in tree order.
    pub fn panels(&self) -> impl DoubleEndedIterator<Item = &PanelId> {
        self.panels.iter()
    }
    /// All logical identities in tree order, starting with the root.
    pub fn nodes(&self) -> impl Iterator<Item = &NodeId> {
        self.nodes.iter()
    }
    /// Direct children in declaration order, or `None` for an unknown group.
    pub fn children(&self, group: &GroupId) -> Option<&[NodeId]> {
        self.children.get(group).map(Vec::as_slice)
    }
    /// Whether this tree contains an identity.
    pub fn contains(&self, node: &NodeId) -> bool {
        node == &NodeId::Group(self.root.clone()) || self.parents.contains_key(node)
    }
    pub(crate) fn anchor(&self, panel: &PanelId, scope: &Scope) -> Result<NodeId, PanelError> {
        let node = NodeId::Panel(panel.clone());
        if !self.contains(&node) {
            return Err(PanelError::UnknownNode(node));
        }
        match scope {
            Scope::Panel => Ok(node),
            Scope::Root => Ok(NodeId::Group(self.root.clone())),
            Scope::Ancestor(levels) => {
                let mut at = node;
                for _ in 0..levels.get() {
                    at = NodeId::Group(
                        self.parents
                            .get(&at)
                            .ok_or_else(|| PanelError::InvalidDepth {
                                panel: Some(panel.clone()),
                                levels: levels.get(),
                            })?
                            .clone(),
                    );
                }
                Ok(at)
            }
            Scope::Group(group) => {
                if !self.children.contains_key(group) {
                    return Err(PanelError::UnknownNode(NodeId::Group(group.clone())));
                }
                let mut at = node;
                while let Some(parent) = self.parents.get(&at) {
                    if parent == group {
                        return Ok(NodeId::Group(group.clone()));
                    }
                    at = NodeId::Group(parent.clone());
                }
                Err(PanelError::NotAncestor {
                    panel: panel.clone(),
                    group: group.clone(),
                })
            }
        }
    }
    /// Partition the supplied panels by scope, preserving tree order.
    ///
    /// Unlisted panels do not join a group. Empty input returns no groups.
    /// Duplicate participants, unknown IDs, and invalid scopes are errors.
    pub fn group(
        &self,
        participants: impl IntoIterator<Item = PanelId>,
        scope: Scope,
    ) -> Result<PanelGroups, PanelError> {
        if let Scope::Group(group) = &scope
            && !self.children.contains_key(group)
        {
            return Err(PanelError::UnknownNode(NodeId::Group(group.clone())));
        }
        let mut selected = BTreeSet::new();
        for panel in participants {
            self.anchor(&panel, &scope)?;
            if !selected.insert(panel.clone()) {
                return Err(PanelError::DuplicateParticipant(panel));
            }
        }
        let mut groups: Vec<PanelGroup> = Vec::new();
        for panel in self.panels().filter(|id| selected.contains(*id)) {
            let anchor = self.anchor(panel, &scope)?;
            if let Some(group) = groups.iter_mut().find(|g| g.anchor == anchor) {
                group.members.push(panel.clone());
            } else {
                groups.push(PanelGroup {
                    anchor,
                    members: vec![panel.clone()],
                });
            }
        }
        Ok(PanelGroups(groups))
    }
}

/// A scope anchor and the explicitly participating panels under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelGroup {
    anchor: NodeId,
    members: Vec<PanelId>,
}
impl PanelGroup {
    /// Logical anchor shared by these participants.
    pub fn anchor(&self) -> &NodeId {
        &self.anchor
    }
    /// Participants in tree order.
    pub fn members(&self) -> &[PanelId] {
        &self.members
    }
}
/// A disjoint grouping result. The caller associates it with its resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelGroups(Vec<PanelGroup>);
impl PanelGroups {
    /// Groups in order of their first participating panel in the tree.
    pub fn iter(&self) -> impl Iterator<Item = &PanelGroup> {
        self.0.iter()
    }
    /// The group containing this participant, if it was supplied.
    pub fn for_panel(&self, panel: &PanelId) -> Option<&PanelGroup> {
        self.0.iter().find(|g| g.members.contains(panel))
    }
}
