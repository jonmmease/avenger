//! The rules that span the messages of a chart: identities and partitions, facets, selections and
//! their clauses, shared scales and projections, and tables of tracks.

mod support;

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, Int64Array, StringArray},
    record_batch::RecordBatch,
};
use avenger_chart::{
    proto::{
        self, guide, node, plot, scale_spec, track, CartesianLayer, Encoding, Node, Plot, ValueRef,
    },
    resolve::{self, Tree},
    Chart,
};
use support::*;

#[test]
fn nodes_with_one_id_have_different_partitions() {
    let row = |a: u32, b: u32| {
        concat(
            proto::Direction::Row,
            vec![
                plot_with("panel", vec![value("sex", a)], vec![symbols(0)]),
                plot_with("panel", vec![value("sex", b)], vec![symbols(0)]),
            ],
        )
    };
    read(parts(row(0, 1), strings(&["female", "male"]))).unwrap();
    invalid(
        parts(row(0, 1), strings(&["female", "female"])),
        "root.concat.children[1].plot",
        "one id and one partition",
    );
}

#[test]
fn a_leaf_gets_one_value_of_each_partition_column() {
    let mut root = concat(
        proto::Direction::Row,
        vec![plot_with("", vec![value("sex", 1)], vec![symbols(0)])],
    );
    concat_mut(&mut root).partition = vec![value("sex", 0)];
    read(parts(root.clone(), strings(&["female", "female"]))).unwrap();
    invalid(
        parts(root, strings(&["female", "male"])),
        "root.concat.children[0].plot.partition",
        "two different values",
    );
}

#[test]
fn partition_values_on_a_node_or_on_its_children_give_one_partition() {
    let mut hoisted = concat(
        proto::Direction::Row,
        vec![plot(vec![symbols(0)]), plot(vec![symbols(0)])],
    );
    concat_mut(&mut hoisted).partition = vec![value("species", 0)];
    let spread = concat(
        proto::Direction::Row,
        vec![
            plot_with("", vec![value("species", 0)], vec![symbols(0)]),
            plot_with("", vec![value("species", 0)], vec![symbols(0)]),
        ],
    );
    for root in [hoisted, spread] {
        let chart = read(parts(root, strings(&["Adelie"]))).unwrap();
        let tree = Tree::new(&chart).unwrap();
        let partition = &tree.partitions["root.concat"];
        assert_eq!(partition.keys().collect::<Vec<_>>(), ["species"]);
    }
}

#[test]
fn the_children_of_a_facet_hold_its_column_and_share_alike() {
    let facet = |children: Vec<Node>| {
        let mut root = concat(proto::Direction::Row, children);
        concat_mut(&mut root).facet = Some(proto::Facet {
            column: "species".to_string(),
            header: None,
        });
        root
    };
    let child = |index| plot_with("", vec![value("species", index)], vec![symbols(0)]);
    read(parts(
        facet(vec![child(0), child(1)]),
        strings(&["Adelie", "Gentoo"]),
    ))
    .unwrap();
    invalid(
        parts(
            facet(vec![child(0), plot(vec![symbols(0)])]),
            strings(&["Adelie"]),
        ),
        "root.concat.children[1]",
        "has the facet's column `species`",
    );
    let mut values = strings(&["Adelie"]);
    values.push(Arc::new(Int64Array::from(vec![2])));
    invalid(
        parts(facet(vec![child(0), child(1)]), values),
        "root.concat.children[1]",
        "one Arrow type",
    );
    let mut independent = child(1);
    plot_mut(&mut independent).resolve = vec![proto::Resolve {
        channel: proto::Channel::Y as i32,
        sharing: proto::Sharing::Independent as i32,
    }];
    invalid(
        parts(
            facet(vec![child(0), independent]),
            strings(&["Adelie", "Gentoo"]),
        ),
        "root.concat.children[1]",
        "share each channel alike",
    );
}

#[test]
fn selections_with_one_name_join_with_equal_resolutions() {
    let mut chart_parts = parts(selecting(), vec![]);
    chart_parts.0.selections = vec![
        brush(proto::SelectionResolution::Global),
        brush(proto::SelectionResolution::Global),
    ];
    read(chart_parts.clone()).unwrap();
    chart_parts.0.selections[1].resolution = proto::SelectionResolution::Union as i32;
    invalid(
        chart_parts,
        "selections[1].resolution",
        "their resolutions differ",
    );
}

#[test]
fn an_initial_clause_names_a_plot_that_selects_its_selection() {
    let clause = |owner: &str, high: u32| proto::Clause {
        owner: owner.to_string(),
        conditions: vec![proto::Condition {
            kind: Some(proto::condition::Kind::Range(proto::ValueRange {
                column: "a".to_string(),
                low: Some(ValueRef { index: 0 }),
                high: Some(ValueRef { index: high }),
                ..Default::default()
            })),
        }],
        partition: vec![],
    };
    let values: Vec<ArrayRef> = vec![
        Arc::new(Float64Array::from(vec![1.0])),
        Arc::new(Float64Array::from(vec![2.0])),
        Arc::new(Float64Array::from(vec![3.0])),
    ];
    let mut chart_parts = parts(selecting(), values);
    let mut selection = brush(proto::SelectionResolution::Global);
    selection.initial = vec![clause("scatter", 1), clause("scatter", 1)];
    chart_parts.0.selections = vec![selection];
    read(chart_parts.clone()).unwrap();
    chart_parts.0.selections[0].initial[1] = clause("scatter", 2);
    invalid(
        chart_parts.clone(),
        "selections[0].initial[1]",
        "differ, which is a conflict",
    );
    chart_parts.0.selections[0].initial = vec![clause("missing", 1)];
    invalid(
        chart_parts,
        "selections[0].initial[0]",
        "no plot or layer with the identity `missing`",
    );
}

#[test]
fn an_owner_writes_one_clause_of_a_selection() {
    let mut node = selecting();
    let tool = plot_mut(&mut node).tools[0].clone();
    plot_mut(&mut node).tools.push(tool);
    let mut chart_parts = parts(node, vec![]);
    chart_parts.0.selections = vec![brush(proto::SelectionResolution::Global)];
    invalid(chart_parts, "root.plot", "two `Select` tools of one owner");
}

#[test]
fn the_specs_of_a_shared_scale_agree() {
    let mut log = symbols(0);
    scale_of(&mut log, true).r#type = Some(scale_spec::Type::Log(proto::LogScale::default()));
    invalid(
        parts(plot(vec![symbols(0), log]), vec![]),
        "root.plot.cartesian.layers[1]",
        "set `type` to different values",
    );

    let format = |pattern: &str| {
        let mut layer = symbols(0);
        scale_of(&mut layer, false).guide = Some(proto::Guide {
            guide: Some(guide::Guide::Axis(proto::Axis {
                labels: Some(proto::GuideLabels {
                    format: pattern.to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        });
        layer
    };
    let mut chart_parts = parts(plot(vec![format(",.0f"), format(".1f")]), vec![]);
    chart_parts.0.number_format_syntax = "d3".to_string();
    invalid(
        chart_parts.clone(),
        "root.plot.cartesian.layers[1]",
        "guide.axis.labels.format",
    );
    chart_parts.0.root = Some(plot(vec![format(",.0f"), symbols(0)]));
    let chart = read(chart_parts).unwrap();
    let tree = Tree::new(&chart).unwrap();
    let groups = resolve::groups(&chart, &tree).unwrap();
    let x = groups
        .iter()
        .find(|group| group.channel == proto::Channel::X)
        .unwrap();
    assert!(
        x.spec.guide.is_some(),
        "the merged spec takes the guide that one spec sets"
    );
}

#[test]
fn a_band_and_a_point_scale_share_a_band_scale() {
    let categories = RecordBatch::try_from_iter([
        (
            "a",
            Arc::new(StringArray::from(vec!["p", "q", "r"])) as ArrayRef,
        ),
        (
            "b",
            Arc::new(Float64Array::from(vec![4.0, 6.0, 5.0])) as ArrayRef,
        ),
    ])
    .unwrap();
    let layer = |scale_type: scale_spec::Type| {
        let mut layer = symbols(0);
        let scale = scale_of(&mut layer, false);
        scale.r#type = Some(scale_type);
        layer
    };
    let band = || layer(scale_spec::Type::Band(proto::BandScale::default()));
    let point = |padding| {
        layer(scale_spec::Type::Point(proto::PointScale {
            padding,
            ..Default::default()
        }))
    };
    let chart = Chart::from_parts(
        chart(plot(vec![point(None), band()])),
        vec![categories.clone()],
        vec![],
    )
    .unwrap();
    let tree = Tree::new(&chart).unwrap();
    let groups = resolve::groups(&chart, &tree).unwrap();
    let x = groups
        .iter()
        .find(|group| group.channel == proto::Channel::X)
        .unwrap();
    assert!(matches!(x.spec.r#type, Some(scale_spec::Type::Band(_))));
    invalid(
        (
            chart_message(vec![point(Some(0.5)), band()]),
            vec![categories],
            vec![],
        ),
        "root.plot.cartesian.layers[1]",
        "`padding` of a point scale",
    );
}

fn chart_message(layers: Vec<CartesianLayer>) -> proto::Chart {
    chart(plot(layers))
}

#[test]
fn explicit_and_data_domains_do_not_share_a_scale() {
    let mut explicit = symbols(0);
    scale_of(&mut explicit, true).domain = Some(proto::Domain {
        domain: Some(proto::domain::Domain::Explicit(ValueRef { index: 0 })),
    });
    let values = vec![avenger_chart::list(Arc::new(Float64Array::from(vec![
        0.0, 10.0,
    ])))];
    invalid(
        parts(plot(vec![symbols(0), explicit.clone()]), values.clone()),
        "root.plot.cartesian.layers[1]",
        "which is a conflict",
    );
    // An independent `y` gives each layer a scale of its own.
    let mut node = plot(vec![symbols(0), explicit]);
    plot_mut(&mut node).resolve = vec![proto::Resolve {
        channel: proto::Channel::Y as i32,
        sharing: proto::Sharing::Independent as i32,
    }];
    read(parts(node, values)).unwrap();
}

#[test]
fn a_plot_has_at_most_two_scales_of_y() {
    let mut node = plot(vec![symbols(0), symbols(0), symbols(0)]);
    plot_mut(&mut node).resolve = vec![proto::Resolve {
        channel: proto::Channel::Y as i32,
        sharing: proto::Sharing::Independent as i32,
    }];
    invalid(
        parts(node, vec![]),
        "root.plot",
        "at most two scales of `y`",
    );
}

#[test]
fn a_family_shares_a_scale_across_independent_plots() {
    let mut a = symbols(0);
    scale_of(&mut a, false).family = "shared".to_string();
    let mut b = symbols(0);
    let scale = scale_of(&mut b, false);
    scale.family = "shared".to_string();
    scale.r#type = Some(scale_spec::Type::Log(proto::LogScale::default()));
    invalid(
        parts(
            concat(proto::Direction::Row, vec![plot(vec![a]), plot(vec![b])]),
            vec![],
        ),
        "root.concat.children[1].plot.cartesian.layers[0]",
        "share a scale",
    );
}

#[test]
fn plots_that_share_x_fix_equal_widths() {
    let sized = |width| {
        let mut node = plot(vec![symbols(0)]);
        plot_mut(&mut node).width = Some(width);
        node
    };
    let mut column = concat(proto::Direction::Column, vec![sized(300.0), sized(400.0)]);
    concat_mut(&mut column).resolve = vec![proto::Resolve {
        channel: proto::Channel::X as i32,
        sharing: proto::Sharing::Shared as i32,
    }];
    invalid(
        parts(column, vec![]),
        "root.concat.children[1].plot",
        "fix different lengths",
    );
    // Without a shared `x`, the column track takes the largest width.
    read(parts(
        concat(proto::Direction::Column, vec![sized(300.0), sized(400.0)]),
        vec![],
    ))
    .unwrap();
}

#[test]
fn the_shares_of_a_track_are_equal() {
    let share = |weight| proto::Track {
        size: Some(track::Size::Share(proto::Share {
            weight: Some(weight),
            max: None,
        })),
    };
    let row = |weight| {
        let mut node = concat(proto::Direction::Row, vec![plot(vec![symbols(0)])]);
        concat_mut(&mut node).layout = Some(proto::ConcatLayout {
            tracks: vec![share(weight)],
            ..Default::default()
        });
        node
    };
    read(parts(
        concat(proto::Direction::Column, vec![row(2.0), row(2.0)]),
        vec![],
    ))
    .unwrap();
    invalid(
        parts(
            concat(proto::Direction::Column, vec![row(2.0), row(3.0)]),
            vec![],
        ),
        "root.concat.children[1].concat",
        "different shares",
    );
}

#[test]
fn a_plot_that_spans_several_tracks_fixes_no_length_along_them() {
    let mut wide = plot(vec![symbols(0)]);
    plot_mut(&mut wide).width = Some(300.0);
    let row = concat(
        proto::Direction::Row,
        vec![plot(vec![symbols(0)]), plot(vec![symbols(0)])],
    );
    invalid(
        parts(concat(proto::Direction::Column, vec![row, wide]), vec![]),
        "root.concat.children[1].plot",
        "spans 2 columns",
    );
}

#[test]
fn map_plots_that_share_a_projection_agree() {
    let map = |projection: proto::spherical::Projection| Node {
        node: Some(node::Node::Plot(Plot {
            coord: Some(plot::Coord::Geo(proto::GeoPlot {
                options: Some(proto::Geo {
                    surface: Some(proto::geo::Surface::Spherical(proto::Spherical {
                        projection: Some(projection),
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                layers: vec![proto::GeoLayer {
                    table: Some(proto::TableRef { index: 0 }),
                    mark: Some(proto::geo_layer::Mark::Symbol(proto::GeoSymbolMark {
                        longitude: Some(Encoding {
                            value: Some(proto::encoding::Value::Column("a".to_string())),
                            ..Default::default()
                        }),
                        latitude: Some(Encoding {
                            value: Some(proto::encoding::Value::Column("b".to_string())),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })),
                    ..Default::default()
                }],
            })),
            ..Default::default()
        })),
    };
    use proto::spherical::Projection;
    let mut row = concat(
        proto::Direction::Row,
        vec![
            map(Projection::Mercator(proto::NoParameters {})),
            map(Projection::Robinson(proto::NoParameters {})),
        ],
    );
    read(parts(row.clone(), vec![])).unwrap();
    concat_mut(&mut row).projection = proto::Sharing::Shared as i32;
    invalid(
        parts(row, vec![]),
        "root.concat.children[1].plot",
        "their surfaces differ",
    );
}
