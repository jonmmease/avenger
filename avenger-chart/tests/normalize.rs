//! The canonical form: every way of writing one chart that the format accepts reads as one chart
//! and writes equal bytes, and reading a canonical chart changes nothing.

mod support;

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array};
use avenger_chart::{
    proto::{self, cartesian_layer, domain_fit, encoding, node, CartesianLayer, Node, ValueRef},
    Chart,
};
use support::*;

/// Reads `a` and `b`, which write one chart in two ways, and checks that they read as one chart
/// and write equal bytes, and that reading either again changes nothing.
#[track_caller]
fn same_chart(a: Parts, b: Parts) -> Chart {
    let a = read(a).unwrap();
    let b = read(b).unwrap();
    assert_eq!(a.message(), b.message());
    assert_eq!(a, b);
    let bytes = a.to_bytes().unwrap();
    assert_eq!(bytes, b.to_bytes().unwrap());
    assert_eq!(
        Chart::from_bytes(bytes.clone())
            .unwrap()
            .to_bytes()
            .unwrap(),
        bytes
    );
    let (message, tables, values) = a.clone().into_parts();
    assert_eq!(Chart::from_parts(message, tables, values).unwrap(), a);
    a
}

fn numbers(values: &[f64]) -> Vec<ArrayRef> {
    (values.iter())
        .map(|value| Arc::new(Float64Array::from(vec![*value])) as ArrayRef)
        .collect()
}

fn row(children: Vec<Node>) -> Node {
    concat(proto::Direction::Row, children)
}

fn layer_mut(node: &mut Node, index: usize) -> &mut CartesianLayer {
    match plot_mut(node).coord.as_mut() {
        Some(proto::plot::Coord::Cartesian(cartesian)) => &mut cartesian.layers[index],
        _ => panic!("the plot is Cartesian"),
    }
}

fn symbol_mut(layer: &mut CartesianLayer) -> &mut proto::SymbolMark {
    match layer.mark.as_mut() {
        Some(cartesian_layer::Mark::Symbol(mark)) => mark,
        _ => panic!("the layer draws symbols"),
    }
}

#[test]
fn partition_values_rise_to_the_highest_node_that_shares_them() {
    // Both plots are Adelie, and only their layers tell the sexes apart.
    let spread = |first: u32, second: u32| {
        let mut left = plot_with("left", vec![value("species", 0)], vec![symbols(0)]);
        layer_mut(&mut left, 0).partition = vec![value("sex", first)];
        let mut right = plot(vec![symbols(0)]);
        layer_mut(&mut right, 0).partition = vec![value("species", 0), value("sex", second)];
        row(vec![left, right])
    };
    let mut hoisted = row(vec![
        plot_with("left", vec![value("sex", 1)], vec![symbols(0)]),
        plot_with("", vec![value("sex", 2)], vec![symbols(0)]),
    ]);
    concat_mut(&mut hoisted).partition = vec![value("species", 0)];
    let values = strings(&["Adelie", "female", "male"]);
    let chart = same_chart(parts(spread(1, 2), values.clone()), parts(hoisted, values));
    let Some(node::Node::Concat(root)) = &chart.root().node else {
        panic!("the root is a concat")
    };
    assert_eq!(root.partition.len(), 1);
    assert_eq!(root.partition[0].column, "species");
    for (index, child) in root.children.iter().enumerate() {
        let Some(node::Node::Plot(plot)) = &child.node else {
            panic!("the child is a plot")
        };
        assert_eq!(plot.partition.len(), 1, "child {index}");
        assert_eq!(plot.partition[0].column, "sex");
        let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
            panic!("the plot is Cartesian")
        };
        assert!(cartesian.layers[0].partition.is_empty());
    }
}

#[test]
fn keyed_lists_follow_their_keys_with_one_entry_for_each() {
    let rule = |channel: proto::Channel, sharing: proto::Sharing| proto::Resolve {
        key: Some(proto::resolve::Key::Channel(channel as i32)),
        sharing: sharing as i32,
    };
    let guide_rule = |part: proto::GuidePart| proto::GuideRule {
        part: part as i32,
        rule: proto::GuideRuleKind::Keep as i32,
    };
    let chart = |resolve: Vec<proto::Resolve>, guide_rules: Vec<proto::GuideRule>| {
        let mut root = row(vec![plot(vec![symbols(0)]), plot(vec![symbols(0)])]);
        concat_mut(&mut root).resolve = resolve;
        concat_mut(&mut root).layout = Some(proto::ConcatLayout {
            guide_rules,
            ..Default::default()
        });
        parts(root, vec![])
    };
    use proto::{Channel::*, GuidePart::*, Sharing::*};
    let chart = same_chart(
        chart(
            vec![rule(Y, Shared), rule(X, Shared), rule(Y, Shared)],
            vec![guide_rule(AxisTitle), guide_rule(AxisTicks)],
        ),
        chart(
            vec![rule(X, Shared), rule(Y, Shared)],
            vec![
                guide_rule(AxisTicks),
                guide_rule(AxisTitle),
                guide_rule(AxisTicks),
            ],
        ),
    );
    let Some(node::Node::Concat(root)) = &chart.root().node else {
        panic!("the root is a concat")
    };
    assert_eq!(root.resolve, vec![rule(X, Shared), rule(Y, Shared)]);
    let rules = &root.layout.as_ref().unwrap().guide_rules;
    assert_eq!(rules, &vec![guide_rule(AxisTicks), guide_rule(AxisTitle)]);
}

#[test]
fn selections_join_and_their_clauses_follow_their_owners() {
    let clause = |owner: &str, high: u32, partition: Vec<proto::PartitionValue>| proto::Clause {
        widget: String::new(),
        owner: owner.to_string(),
        conditions: vec![proto::Condition {
            kind: Some(proto::condition::Kind::Range(proto::ValueRange {
                column: "a".to_string(),
                low: Some(ValueRef { index: 0 }),
                high: Some(ValueRef { index: high }),
                ..Default::default()
            })),
        }],
        partition,
    };
    let owner = |id: &str| {
        let mut node = selecting();
        let plot = plot_mut(&mut node);
        plot.id = id.to_string();
        plot.partition = vec![value("sex", 3), value("species", 4)];
        node
    };
    let root = || row(vec![owner("first"), owner("second")]);
    let mut values = numbers(&[1.0, 2.0, 3.0]);
    values.extend(strings(&["female", "Adelie"]));
    let partition = || vec![value("sex", 3), value("species", 4)];
    let reversed = || vec![value("species", 4), value("sex", 3)];
    let other = proto::Selection {
        name: "another".to_string(),
        resolution: proto::SelectionResolution::Union as i32,
        ..Default::default()
    };

    let mut a = parts(root(), values.clone());
    let mut selection = brush(proto::SelectionResolution::Union);
    selection.initial = vec![
        clause("second", 2, partition()),
        clause("first", 1, partition()),
    ];
    a.0.selections = vec![selection, other.clone()];

    let mut b = parts(root(), values);
    let mut first = brush(proto::SelectionResolution::Union);
    first.initial = vec![clause("first", 1, reversed())];
    let mut second = brush(proto::SelectionResolution::Union);
    second.initial = vec![
        clause("second", 2, partition()),
        clause("first", 1, partition()),
    ];
    b.0.selections = vec![other, first, second];

    let chart = same_chart(a, b);
    let names: Vec<&str> = (chart.selections().iter())
        .map(|selection| selection.name.as_str())
        .collect();
    assert_eq!(names, ["another", "brush"]);
    let owners: Vec<&str> = (chart.selections()[1].initial.iter())
        .map(|clause| clause.owner.as_str())
        .collect();
    assert_eq!(owners, ["first", "second"]);
    let columns: Vec<&str> = (chart.selections()[1].initial[0].partition.iter())
        .map(|value| value.column.as_str())
        .collect();
    assert_eq!(columns, ["sex", "species"]);
}

#[test]
fn equal_tables_merge_and_follow_their_first_layer() {
    let other = table(&[("a", vec![0.0]), ("b", vec![1.0])]);
    let root = |first: u32, second: u32| {
        row(vec![
            plot(vec![symbols(first)]),
            plot(vec![symbols(second)]),
        ])
    };
    let chart = same_chart(
        (chart(root(0, 0)), vec![points()], vec![]),
        (chart(root(2, 1)), vec![other, points(), points()], vec![]),
    );
    assert_eq!(chart.tables(), &[points()]);
}

#[test]
fn ignored_fields_are_left_out() {
    let chart = |ignored: bool| {
        let mut inner = row(vec![plot(vec![symbols(0)])]);
        let mut cell = plot(vec![symbols(0)]);
        let mut layer = symbols(0);
        let mark = symbol_mut(&mut layer);
        mark.size = Some(proto::Encoding {
            value: Some(encoding::Value::Column("a".to_string())),
            title: ignored.then(|| "size".to_string()),
            ..Default::default()
        });
        mark.color = Some(column("b", linear_data()));
        if ignored {
            concat_mut(&mut inner).layout = Some(proto::ConcatLayout {
                horizontal_distribute: proto::Distribute::Center as i32,
                ..Default::default()
            });
            plot_mut(&mut cell).width = Some(200.0);
        }
        let wrap = Node {
            node: Some(node::Node::Wrap(proto::Wrap {
                children: vec![cell],
                ..Default::default()
            })),
        };
        let mut root = concat(
            proto::Direction::Column,
            vec![inner, wrap, plot(vec![layer])],
        );
        concat_mut(&mut root).layout = Some(proto::ConcatLayout {
            vertical_distribute: proto::Distribute::End as i32,
            ..Default::default()
        });
        parts(root, vec![])
    };
    let chart = same_chart(chart(true), chart(false));
    let Some(node::Node::Concat(root)) = &chart.root().node else {
        panic!("the root is a concat")
    };
    // The outermost concat of the table distributes its free length.
    let layout = root.layout.as_ref().unwrap();
    assert_eq!(layout.vertical_distribute, proto::Distribute::End as i32);
}

#[test]
fn options_at_every_default_are_left_out() {
    let chart = |empty: bool| {
        let mut left = plot(vec![symbols(0)]);
        let mut layer = symbols(0);
        let mut root = if empty {
            let x = symbol_mut(&mut layer)
                .x
                .as_mut()
                .unwrap()
                .scale
                .as_mut()
                .unwrap();
            *data_fit(x) = Some(proto::DomainFit {
                fit: Some(domain_fit::Fit::Interval(proto::IntervalFit::default())),
            });
            let y = symbol_mut(&mut layer)
                .y
                .as_mut()
                .unwrap()
                .scale
                .as_mut()
                .unwrap();
            *data_fit(y) = Some(proto::DomainFit::default());
            if let Some(proto::plot::Coord::Cartesian(cartesian)) = &mut plot_mut(&mut left).coord {
                cartesian.options = Some(proto::Cartesian::default());
            }
            let mut root = row(vec![left, plot(vec![layer])]);
            concat_mut(&mut root).layout = Some(proto::ConcatLayout::default());
            root
        } else {
            row(vec![left, plot(vec![layer])])
        };
        concat_mut(&mut root).id = "grid".to_string();
        parts(root, vec![])
    };
    same_chart(chart(true), chart(false));
}

#[test]
fn a_concat_without_children_keeps_its_partition_values() {
    let empty_concat = |partition| {
        let mut node = row(vec![]);
        concat_mut(&mut node).partition = partition;
        node
    };
    let root = row(vec![
        plot_with("", vec![value("sex", 0)], vec![symbols(0)]),
        empty_concat(vec![value("sex", 1)]),
    ]);
    let values = strings(&["female", "male"]);
    let chart = same_chart(parts(root.clone(), values.clone()), parts(root, values));
    let Some(node::Node::Concat(root)) = &chart.root().node else {
        panic!("the root is a concat")
    };
    assert!(root.partition.is_empty());
}

#[test]
fn a_repeated_class_counts_once() {
    let chart = |repeated: bool| {
        let classes = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
        let listed: Vec<String> = match repeated {
            true => classes(&["a", "b", "a"]),
            false => classes(&["a", "b"]),
        };
        let mut layer = symbols(0);
        layer.classes = listed.clone();
        let mut root = plot(vec![layer]);
        plot_mut(&mut root).classes = listed;
        parts(root, vec![])
    };
    same_chart(chart(true), chart(false));
}
