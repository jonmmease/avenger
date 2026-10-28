//! Walks over the node tree of a chart.

use crate::proto::{self, node::Node};

/// The children of a node: those of an overlay, a concat, or a wrap, and none for a unit or an
/// empty child.
pub(crate) fn children(node: &proto::Node) -> &[proto::Node] {
    match &node.node {
        Some(Node::Overlay(overlay)) => &overlay.children,
        Some(Node::Concat(concat)) => &concat.children,
        Some(Node::Wrap(wrap)) => &wrap.children,
        _ => &[],
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

/// Calls `f` with each node at or below `node`, in tree order, with its path, which starts with
/// `path`.
pub(crate) fn visit<'a>(
    node: &'a proto::Node,
    path: &str,
    f: &mut dyn FnMut(&'a proto::Node, &str),
) {
    f(node, path);
    for (index, child) in children(node).iter().enumerate() {
        visit(child, &format!("{path}/children[{index}]"), f);
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
