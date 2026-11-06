//! The helpers of producers: parts that hand out references, fragments, and the combination of
//! charts.

mod support;

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, AsArray, Float64Array, StringArray},
    datatypes::Float64Type,
    ipc::{writer::IpcWriteOptions, MetadataVersion},
};
use avenger_chart::{
    build::{self, ChartParts, Fragment},
    list_value,
    proto::{self, node::Node, plot::Coord},
    values_from_ipc, values_to_ipc, Chart, ChartError,
};
use support::*;

fn numbers(values: Vec<f64>) -> ArrayRef {
    Arc::new(Float64Array::from(values))
}

fn text(value: &str) -> ArrayRef {
    Arc::new(StringArray::from(vec![value]))
}

/// The explicit domain of the `x` of the symbols of `layer`.
fn explicit_x(layer: &mut proto::CartesianLayer, domain: proto::ValueRef) {
    *scale_of(layer, false) = linear(proto::domain::Domain::Explicit(domain));
}

/// A scatter plot whose plot declares `partition`, with `id`.
fn scatter(id: &str, partition: Vec<(&str, &str)>) -> Chart {
    let mut parts = ChartParts::new();
    let table = parts.table(points());
    let mut node = plot_with(id, vec![], vec![symbols(table.index)]);
    plot_mut(&mut node).partition = (partition.into_iter())
        .map(|(column, value)| parts.partition(column, text(value)))
        .collect();
    parts.finish(build::chart(node)).unwrap()
}

#[track_caller]
fn fails(result: Result<Chart, ChartError>, path: &str, reason: &str) {
    match result {
        Err(ChartError::Invalid {
            path: at,
            reason: why,
        }) => {
            assert_eq!(at, path, "the error names the path ({why})");
            assert!(why.contains(reason), "`{why}` says `{reason}`");
        }
        other => panic!("expected an error at {path}, got {other:?}"),
    }
}

fn row() -> Fragment<proto::Concat> {
    Fragment::new(proto::Concat {
        direction: proto::Direction::Row as i32,
        ..Default::default()
    })
}

#[test]
fn parts_hand_out_references_to_their_tables_and_values() {
    let mut parts = ChartParts::new();
    let table = parts.table(points());
    let domain = parts.list(numbers(vec![0.0, 10.0]));
    let mut layer = symbols(table.index);
    explicit_x(&mut layer, domain);
    layer.partition = vec![parts.partition("sex", text("female"))];
    let chart = parts
        .finish(build::chart(build::cartesian(vec![layer])))
        .unwrap();
    let Some(Node::Plot(plot)) = &chart.root().node else {
        panic!("the root is a plot")
    };
    let Some(Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("the plot is Cartesian")
    };
    let Some(proto::cartesian_layer::Mark::Symbol(mark)) = &cartesian.layers[0].mark else {
        panic!("the layer draws symbols")
    };
    let x = mark.x.as_ref().unwrap().scale.as_ref().unwrap();
    let Some(proto::domain::Domain::Explicit(domain)) = x.domain.as_ref().unwrap().domain else {
        panic!("the domain is explicit")
    };
    let ends = chart.list_elements(&domain);
    assert_eq!(ends.as_primitive::<Float64Type>().values(), &[0.0, 10.0]);
    assert!(chart.resolve().tree.partitions["root.plot"].contains_key("sex"));
}

#[test]
fn an_embedded_fragment_refers_to_its_own_values() {
    let fragment = |low: f64| {
        Fragment::with_values(
            linear(proto::domain::Domain::Explicit(proto::ValueRef {
                index: 0,
            })),
            vec![list_value(numbers(vec![low, low + 1.0]))],
        )
    };
    let mut parts = ChartParts::new();
    parts.value(text("taken"));
    let first = parts.embed(fragment(0.0));
    let second = parts.embed(fragment(5.0));
    let low = |spec: &proto::ScaleSpec| {
        let Some(proto::domain::Domain::Explicit(value)) = spec.domain.as_ref().unwrap().domain
        else {
            panic!("the domain is explicit")
        };
        let list = parts.values()[value.index as usize]
            .as_list::<i32>()
            .value(0);
        list.as_primitive::<Float64Type>().value(0)
    };
    assert_eq!((low(&first), low(&second)), (0.0, 5.0));
}

#[test]
fn values_round_trip_through_their_stream() {
    let values = vec![numbers(vec![1.0]), text("a"), numbers(vec![2.0])];
    let decoded = values_from_ipc(&values_to_ipc(&values).unwrap()).unwrap();
    assert_eq!(decoded.len(), 3);
    for (decoded, value) in decoded.iter().zip(&values) {
        assert_eq!(decoded.as_ref(), value.as_ref());
    }
    assert!(values_from_ipc(&values_to_ipc(&[]).unwrap())
        .unwrap()
        .is_empty());
}

#[test]
fn a_concat_copies_the_streams_of_its_children() {
    let options = IpcWriteOptions::try_new(8, false, MetadataVersion::V5).unwrap();
    let stream = ipc(&points(), options.clone());
    let read = || {
        let mut message = chart(plot(vec![symbols(0)]));
        message.tables = vec![proto::Table {
            padding: Default::default(),
            ipc: ipc(&points(), options.clone()).into(),
        }];
        Chart::from_message(message).unwrap()
    };
    let combined = build::concat(row(), vec![read(), read()]).unwrap();
    // Two copies of one table are one table, written as its stream.
    assert_eq!(combined.tables().len(), 1);
    let bytes = combined.to_bytes().unwrap();
    assert!(bytes
        .windows(stream.len())
        .any(|window| window == stream.as_slice()));
}

#[test]
fn format_syntaxes_merge_and_outer_sizes_drop() {
    let with = |syntax: &str, width: Option<f32>| {
        let (mut message, tables, values) = scatter("", vec![]).into_parts();
        message.number_format_syntax = syntax.to_string();
        message.width = width;
        Chart::from_parts(message, tables, values).unwrap()
    };
    let combined = build::concat(row(), vec![with("d3", Some(300.0)), with("", None)]).unwrap();
    assert_eq!(combined.message().number_format_syntax, "d3");
    assert_eq!(combined.message().width, None);
    fails(
        build::concat(row(), vec![with("d3", None), with("icu", None)]),
        "children[1].number_format_syntax",
        "which is a conflict",
    );
}

#[test]
fn selections_with_one_name_join() {
    let selecting = |id: &str| {
        let mut node = selecting();
        plot_mut(&mut node).id = id.to_string();
        let mut message = chart(node);
        message.selections = vec![brush(proto::SelectionResolution::Global)];
        Chart::from_parts(message, vec![points()], vec![]).unwrap()
    };
    let combined = build::concat(row(), vec![selecting("a"), selecting("b")]).unwrap();
    assert_eq!(combined.selections().len(), 1);
}

#[test]
fn a_wrapped_facet_keeps_the_partitions_of_its_children() {
    let node = Fragment::new(proto::Wrap {
        facet: Some(proto::Facet {
            column: "genre".to_string(),
            header: None,
        }),
        ..Default::default()
    });
    let children = vec![
        scatter("panel", vec![("genre", "Drama")]),
        scatter("panel", vec![("genre", "Comedy")]),
    ];
    let combined = build::wrap(node, children).unwrap();
    let resolved = combined.resolve();
    for index in 0..2 {
        let path = format!("root.wrap.children[{index}].plot");
        assert!(resolved.tree.partitions[&path].contains_key("genre"));
    }
}

#[test]
fn a_facet_grid_places_each_chart_in_its_cell() {
    let node = Fragment::new(proto::Concat::default());
    let children = vec![
        scatter("panel", vec![("sex", "female"), ("island", "Biscoe")]),
        scatter("panel", vec![("sex", "male"), ("island", "Dream")]),
    ];
    let grid = build::facet_grid("sex", "island", node, children).unwrap();
    let Some(Node::Concat(column)) = &grid.root().node else {
        panic!("the root is a concat")
    };
    assert_eq!(column.direction, proto::Direction::Column as i32);
    assert_eq!(column.facet.as_ref().unwrap().column, "sex");
    let cells: Vec<Vec<&str>> = (column.children.iter())
        .map(|row| {
            let Some(Node::Concat(row)) = &row.node else {
                panic!("each row is a concat")
            };
            assert_eq!(row.facet.as_ref().unwrap().column, "island");
            (row.children.iter())
                .map(|cell| match cell.node {
                    Some(Node::Plot(_)) => "plot",
                    Some(Node::Empty(_)) => "empty",
                    _ => "other",
                })
                .collect()
        })
        .collect();
    assert_eq!(cells, [["plot", "empty"], ["empty", "plot"]]);
    // An empty cell holds the values of its row and its column, so its header labels it.
    let resolved = grid.resolve();
    let empty = &resolved.tree.partitions["root.concat.children[0].concat.children[1].empty"];
    let value = |column: &str| empty[column].as_string::<i32>().value(0).to_string();
    assert_eq!(
        (value("sex"), value("island")),
        ("female".into(), "Dream".into())
    );

    fails(
        build::facet_grid(
            "sex",
            "island",
            Fragment::new(proto::Concat::default()),
            vec![scatter("panel", vec![("sex", "female")])],
        ),
        "children[0]",
        "no partition value of `island`",
    );
    fails(
        build::facet_grid(
            "sex",
            "island",
            Fragment::new(proto::Concat::default()),
            vec![
                scatter("a", vec![("sex", "female"), ("island", "Biscoe")]),
                scatter("b", vec![("sex", "female"), ("island", "Biscoe")]),
            ],
        ),
        "children[1]",
        "stand in one cell",
    );
}

#[test]
fn an_overlay_draws_the_layers_of_its_charts_in_one_plot() {
    let titled = |title: Option<&str>, partition: Vec<(&str, &str)>| {
        let (mut message, tables, values) = scatter("", partition).into_parts();
        if let Some(Node::Plot(plot)) = message.root.as_mut().and_then(|root| root.node.as_mut()) {
            plot.title = title.map(|text| proto::Title {
                text: text.to_string(),
                ..Default::default()
            });
        }
        Chart::from_parts(message, tables, values).unwrap()
    };
    let node = || Fragment::new(proto::Plot::default());
    let overlaid = build::overlay(
        node(),
        vec![
            titled(Some("Mass"), vec![("sex", "female")]),
            titled(None, vec![("sex", "male")]),
        ],
    )
    .unwrap();
    let Some(Node::Plot(plot)) = &overlaid.root().node else {
        panic!("the root is a plot")
    };
    assert_eq!(plot.title.as_ref().unwrap().text, "Mass");
    let Some(Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("the plot is Cartesian")
    };
    assert_eq!(cartesian.layers.len(), 2);
    // The plots' partitions differ, so each layer declares its own.
    assert!(plot.partition.is_empty());
    assert!(cartesian
        .layers
        .iter()
        .all(|layer| layer.partition.len() == 1));

    let titled_node = proto::Plot {
        title: Some(proto::Title {
            text: "Both".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let overlaid = build::overlay(
        Fragment::new(titled_node),
        vec![titled(Some("A"), vec![]), titled(Some("B"), vec![])],
    )
    .unwrap();
    let Some(Node::Plot(plot)) = &overlaid.root().node else {
        panic!("the root is a plot")
    };
    assert_eq!(plot.title.as_ref().unwrap().text, "Both");
    fails(
        build::overlay(
            node(),
            vec![titled(Some("A"), vec![]), titled(Some("B"), vec![])],
        ),
        "children.title",
        "which is a conflict",
    );

    let polar = {
        let mut parts = ChartParts::new();
        let table = parts.table(points());
        let layer = proto::PolarLayer {
            table: Some(table),
            mark: Some(proto::polar_layer::Mark::Symbol(proto::PolarSymbolMark {
                theta: Some(column("a", linear_data())),
                radius: Some(column("b", linear_data())),
                ..Default::default()
            })),
            ..Default::default()
        };
        parts
            .finish(build::chart(build::polar(vec![layer])))
            .unwrap()
    };
    fails(
        build::overlay(node(), vec![titled(None, vec![]), polar]),
        "children[1]",
        "of one coordinate system",
    );
}

#[test]
fn a_finished_chart_takes_a_partition_value() {
    let chart = build::with_partition(scatter("", vec![]), "sex", text("female")).unwrap();
    let partition = &chart.resolve().tree.partitions["root.plot"];
    assert_eq!(partition["sex"].as_string::<i32>().value(0), "female");
    assert_eq!(partition["sex"].len(), 1);
}
