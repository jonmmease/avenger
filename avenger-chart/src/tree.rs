//! Walks over the node tree of a chart and over the fields that refer to its values.

use crate::proto::{
    self, axis, category_fit, condition, domain, domain_fit, encoding, guide, node::Node, range,
    tuple_field,
};

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

/// Calls `f` with each index of a value in `chart`, in tree order: the nodes from the root, and
/// then the initial contributions of the selections.
pub(crate) fn for_each_value_mut(chart: &mut proto::Chart, f: &mut dyn FnMut(&mut u32)) {
    if let Some(root) = &mut chart.root {
        node_values(root, f);
    }
    for selection in &mut chart.selections {
        for contribution in &mut selection.initial {
            for value in &mut contribution.partition {
                f(&mut value.value);
            }
            for field in contribution
                .tuples
                .iter_mut()
                .flat_map(|tuple| &mut tuple.fields)
            {
                match &mut field.test {
                    Some(tuple_field::Test::Set(set)) => f(&mut set.values),
                    Some(tuple_field::Test::Range(range)) => f(&mut range.ends),
                    Some(tuple_field::Test::Polygon(polygon)) => f(&mut polygon.vertices),
                    Some(tuple_field::Test::LongitudeRange(_)) | None => {}
                }
            }
        }
    }
}

fn node_values(node: &mut proto::Node, f: &mut dyn FnMut(&mut u32)) {
    for value in partition_mut(node).into_iter().flatten() {
        f(&mut value.value);
    }
    if let Some(Node::Unit(unit)) = &mut node.node {
        for encoding in &mut unit.encodings {
            if let Some(encoding::Value::Constant(index) | encoding::Value::ScaledConstant(index)) =
                &mut encoding.value
            {
                f(index);
            }
            for condition in &mut encoding.conditions {
                if let Some(
                    condition::Value::Constant(index) | condition::Value::ScaledConstant(index),
                ) = &mut condition.value
                {
                    f(index);
                }
            }
            if let Some(spec) = &mut encoding.scale {
                spec_values(spec, f);
            }
        }
    }
    for child in children_mut(node).into_iter().flatten() {
        node_values(child, f);
    }
}

fn spec_values(spec: &mut proto::ScaleSpec, f: &mut dyn FnMut(&mut u32)) {
    if let Some(domain) = &mut spec.domain {
        domain_values(domain, f);
    }
    match spec.fit.as_mut().and_then(|fit| fit.fit.as_mut()) {
        Some(domain_fit::Fit::Interval(interval)) => {
            if let Some(empty) = &mut interval.empty {
                f(empty);
            }
        }
        Some(domain_fit::Fit::Categories(categories)) => {
            if let Some(category_fit::Order::Preferred(index)) = &mut categories.order {
                f(index);
            }
        }
        None => {}
    }
    if let Some(range::Range::Values(index)) =
        spec.range.as_mut().and_then(|range| range.range.as_mut())
    {
        f(index);
    }
    match spec.guide.as_mut().and_then(|guide| guide.guide.as_mut()) {
        Some(guide::Guide::Axis(axis)) => axis_values(axis, f),
        Some(guide::Guide::Legend(legend)) => {
            if let Some(values) = &mut legend.values {
                f(values);
            }
        }
        Some(guide::Guide::Colorbar(colorbar)) => {
            if let Some(axis) = &mut colorbar.axis {
                axis_values(axis, f);
            }
        }
        Some(guide::Guide::None(_)) | None => {}
    }
}

fn domain_values(domain: &mut proto::Domain, f: &mut dyn FnMut(&mut u32)) {
    match &mut domain.domain {
        Some(domain::Domain::Explicit(index) | domain::Domain::Summary(index)) => f(index),
        Some(domain::Domain::Selection(selection)) => {
            if let Some(fallback) = &mut selection.fallback {
                domain_values(fallback, f);
            }
        }
        Some(domain::Domain::Data(_)) | None => {}
    }
}

fn axis_values(axis: &mut proto::Axis, f: &mut dyn FnMut(&mut u32)) {
    if let Some(axis::Ticks::TickValues(index)) = &mut axis.ticks {
        f(index);
    }
}
