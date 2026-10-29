//! Walks over the node tree of a chart and over the fields that refer to its values and its
//! tables.

use crate::proto::{self, node::Node};

/// A message that holds `ValueRef`s, directly or through its fields. build.rs implements it for
/// each such message.
pub(crate) trait VisitValues {
    /// Calls `f` with each `ValueRef` of the message, in the order that the `.proto` files declare
    /// the fields, depth first, where a oneof takes the place of its first member.
    fn visit_values_mut(&mut self, f: &mut dyn FnMut(&mut proto::ValueRef));
}

/// A message that holds `TableRef`s, directly or through its fields. build.rs implements it for
/// each such message.
pub(crate) trait VisitTables {
    /// Calls `f` with each `TableRef` of the message, in the order of `VisitValues`.
    fn visit_tables_mut(&mut self, f: &mut dyn FnMut(&mut proto::TableRef));
}

include!(concat!(env!("OUT_DIR"), "/values.rs"));

/// The partition values that a node declares.
pub fn partition(node: &proto::Node) -> &[proto::PartitionValue] {
    match &node.node {
        Some(Node::Unit(unit)) => &unit.partition,
        Some(Node::Overlay(overlay)) => &overlay.partition,
        Some(Node::Concat(concat)) => &concat.partition,
        Some(Node::Wrap(wrap)) => &wrap.partition,
        Some(Node::Empty(empty)) => &empty.partition,
        None => &[],
    }
}

/// The partition values that a node declares, to change.
pub(crate) fn partition_mut(node: &mut proto::Node) -> Option<&mut Vec<proto::PartitionValue>> {
    match &mut node.node {
        Some(Node::Unit(unit)) => Some(&mut unit.partition),
        Some(Node::Overlay(overlay)) => Some(&mut overlay.partition),
        Some(Node::Concat(concat)) => Some(&mut concat.partition),
        Some(Node::Wrap(wrap)) => Some(&mut wrap.partition),
        Some(Node::Empty(empty)) => Some(&mut empty.partition),
        None => None,
    }
}
