//! Panel hierarchy, sharing policies, and guide placement.
//!
//! [`PanelTree::group`] resolves logical sharing. [`PanelTree::arrange`]
//! assigns physical slots without changing that hierarchy.
//! [`PanelTree::plan_guides`] uses solved [`PanelFrames`] and caller-supplied
//! equivalence evidence to select guides and their anchors.
//!
//! Domain aggregation, guide content, measurement, layout iteration, and
//! rendering remain caller responsibilities. See the crate README for the
//! complete lifecycle and the native/WASM small-multiples explorer.
//!
//! ```
//! use avenger_panels::{PanelNode, PanelTree, Scope};
//! let tree = PanelTree::new("figure".into(), [
//!     PanelNode::group("north", [PanelNode::panel("a"), PanelNode::panel("b")]),
//!     PanelNode::group("south", [PanelNode::panel("c")]),
//! ])?;
//! let groups = tree.group(tree.panels().cloned(), Scope::ancestor(1)?)?;
//! assert_eq!(groups.iter().count(), 2);
//! # Ok::<(), avenger_panels::PanelError>(())
//! ```
#![warn(missing_docs)]

mod arrangement;
mod error;
mod frames;
mod guides;
mod identity;
mod tree;

pub use arrangement::{
    ArrangementSpec, GroupArrangement, PanelArrangement, PanelDisplay, PanelGrid,
};
pub use avenger_layout::{CellAlign, GridShape, GridSlot, Rect, Side};
pub use error::PanelError;
pub use frames::PanelFrames;
pub use guides::{
    AxisLabels, DecisionReason, GuideContribution, GuideDecision, GuideInstance, GuideInstanceId,
    GuideKind, GuideOptions, GuidePlan, GuideRequest, LabelVisibility, SharedGuide,
    SharedGuideKind,
};
pub use identity::{EquivalenceKey, GroupId, GuideKey, NodeId, PanelId};
pub use tree::{PanelGroup, PanelGroups, PanelNode, PanelTree, Scope};
