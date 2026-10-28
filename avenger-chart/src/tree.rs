//! Walks over the node tree of a chart.

use crate::proto::{self, node::Node};

/// The partition values that a node declares.
pub(crate) fn partition(node: &proto::Node) -> &[proto::PartitionValue] {
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

fn children_mut(node: &mut proto::Node) -> Option<&mut Vec<proto::Node>> {
    match &mut node.node {
        Some(Node::Overlay(overlay)) => Some(&mut overlay.children),
        Some(Node::Concat(concat)) => Some(&mut concat.children),
        Some(Node::Wrap(wrap)) => Some(&mut wrap.children),
        _ => None,
    }
}

/// Calls `f` with each unit at or below `node`, in tree order.
pub(crate) fn for_each_unit_mut(node: &mut proto::Node, f: &mut dyn FnMut(&mut proto::Unit)) {
    if let Some(Node::Unit(unit)) = &mut node.node {
        f(unit);
    }
    if let Some(children) = children_mut(node) {
        for child in children {
            for_each_unit_mut(child, f);
        }
    }
}
