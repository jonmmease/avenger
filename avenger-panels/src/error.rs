use crate::{GroupId, GuideKey, NodeId, PanelId, Rect};
use std::fmt;

/// An invalid tree, arrangement, frame set, or guide request.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PanelError {
    /// An identity occurs more than once.
    DuplicateNode(NodeId),
    /// An identity is absent from the tree.
    UnknownNode(NodeId),
    /// A panel participates twice in one request.
    DuplicateParticipant(PanelId),
    /// An ancestor depth is zero or exceeds a panel's ancestry.
    InvalidDepth {
        /// Panel being resolved, if any.
        panel: Option<PanelId>,
        /// Requested depth.
        levels: usize,
    },
    /// A requested group is not an ancestor of this panel.
    NotAncestor {
        /// Participating panel.
        panel: PanelId,
        /// Requested group.
        group: GroupId,
    },
    /// A physical grid cannot represent the group's children.
    InvalidArrangement {
        /// Affected group.
        group: GroupId,
        /// Specific validation failure.
        reason: String,
    },
    /// A frame is missing for a logical node.
    MissingFrame(NodeId),
    /// Geometry is nonfinite, negative, or has overflowing endpoints.
    InvalidFrame {
        /// Affected node.
        node: NodeId,
        /// Supplied rectangle.
        rect: Rect,
    },
    /// Frames describe a different logical tree.
    MismatchedTree,
    /// Two guide requests use the same family key.
    DuplicateGuide(GuideKey),
    /// Several shared contributions lack matching equivalence evidence.
    IncompatibleGuide {
        /// Guide family.
        key: GuideKey,
        /// Requested placement anchor.
        anchor: NodeId,
        /// Conflicting eligible participants.
        panels: Vec<PanelId>,
    },
    /// Alignment tolerance is negative or nonfinite.
    InvalidTolerance(f32),
}

impl fmt::Display for PanelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode(id) => write!(f, "Duplicate identity: {id}"),
            Self::UnknownNode(id) => write!(f, "Unknown identity: {id}"),
            Self::DuplicateParticipant(id) => write!(f, "Panel {id} participates more than once"),
            Self::InvalidDepth { panel, levels } => {
                write!(f, "Invalid ancestor depth {levels} for {panel:?}")
            }
            Self::NotAncestor { panel, group } => {
                write!(f, "Group {group} is not an ancestor of panel {panel}")
            }
            Self::InvalidArrangement { group, reason } => {
                write!(f, "Invalid arrangement for group {group}: {reason}")
            }
            Self::MissingFrame(id) => write!(f, "Missing frame for {id}"),
            Self::InvalidFrame { node, rect } => write!(f, "Invalid frame for {node}: {rect:?}"),
            Self::MismatchedTree => {
                f.write_str("Frames and planner describe different logical trees")
            }
            Self::DuplicateGuide(key) => write!(f, "Duplicate guide family: {key}"),
            Self::IncompatibleGuide {
                key,
                anchor,
                panels,
            } => write!(
                f,
                "Guide {key} at {anchor} requires equivalent contributions: {panels:?}"
            ),
            Self::InvalidTolerance(value) => write!(
                f,
                "Alignment tolerance must be finite and nonnegative: {value}"
            ),
        }
    }
}
impl std::error::Error for PanelError {}
