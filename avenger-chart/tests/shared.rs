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
        self, guide, node, plot, scale, track, CartesianLayer, Encoding, Node, Plot, ValueRef,
    },
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
        let resolved = chart.resolve();
        let partition = &resolved.tree.partitions["root.concat"];
        assert_eq!(partition.keys().collect::<Vec<_>>(), ["species"]);
        // The two plots stand in the two column tracks of one table.
        let place = |index: usize| {
            let path = format!("root.concat.children[{index}]");
            resolved.tables.place(&path).unwrap().clone()
        };
        assert_eq!((place(0).table, place(0).tracks[0].clone()), (0, 0..1));
        assert_eq!((place(1).table, place(1).tracks[0].clone()), (0, 1..2));
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
        key: Some(proto::resolve::Key::Channel(proto::Channel::Y as i32)),
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
    scale_of(&mut log, true).r#type = Some(scale::Type::Log(proto::LogScale::default()));
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
    let groups = chart.resolve().scales;
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
    let layer = |scale_type: scale::Type| {
        let mut layer = symbols(0);
        let scale = scale_of(&mut layer, false);
        scale.r#type = Some(scale_type);
        layer
    };
    let band = || layer(scale::Type::Band(proto::BandScale::default()));
    let point = |padding| {
        layer(scale::Type::Point(proto::PointScale {
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
    let groups = chart.resolve().scales;
    let x = groups
        .iter()
        .find(|group| group.channel == proto::Channel::X)
        .unwrap();
    assert!(matches!(x.spec.r#type, Some(scale::Type::Band(_))));
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
    let values = vec![avenger_chart::list_value(Arc::new(Float64Array::from(
        vec![0.0, 10.0],
    )))];
    invalid(
        parts(plot(vec![symbols(0), explicit.clone()]), values.clone()),
        "root.plot.cartesian.layers[1]",
        "which is a conflict",
    );
    // An independent `y` gives each layer a scale of its own.
    let mut node = plot(vec![symbols(0), explicit]);
    plot_mut(&mut node).resolve = vec![proto::Resolve {
        key: Some(proto::resolve::Key::Channel(proto::Channel::Y as i32)),
        sharing: proto::Sharing::Independent as i32,
    }];
    read(parts(node, values)).unwrap();
}

#[test]
fn a_plot_has_at_most_two_scales_of_y() {
    let mut node = plot(vec![symbols(0), symbols(0), symbols(0)]);
    plot_mut(&mut node).resolve = vec![proto::Resolve {
        key: Some(proto::resolve::Key::Channel(proto::Channel::Y as i32)),
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
    scale.r#type = Some(scale::Type::Log(proto::LogScale::default()));
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
        key: Some(proto::resolve::Key::Channel(proto::Channel::X as i32)),
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
    concat_mut(&mut row).projection = Some(proto::Sharing::Shared as i32);
    invalid(
        parts(row, vec![]),
        "root.concat.children[1].plot",
        "their surfaces differ",
    );
}

/// A row of `children` that shares `x`.
fn shared_row(children: Vec<Node>) -> Node {
    let mut row = concat(proto::Direction::Row, children);
    concat_mut(&mut row).resolve = vec![proto::Resolve {
        key: Some(proto::resolve::Key::Channel(proto::Channel::X as i32)),
        sharing: proto::Sharing::Shared as i32,
    }];
    row
}

fn plot_layer_mut(node: &mut Node, index: usize) -> &mut CartesianLayer {
    match plot_mut(node).coord.as_mut() {
        Some(plot::Coord::Cartesian(cartesian)) => &mut cartesian.layers[index],
        _ => panic!("the plot is Cartesian"),
    }
}

#[test]
fn the_clauses_of_joined_selections_agree() {
    let clause = |high: u32| proto::Clause {
        widget: String::new(),
        owner: "scatter".to_string(),
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
    let values: Vec<ArrayRef> = (1..=3)
        .map(|value| Arc::new(Float64Array::from(vec![value as f64])) as ArrayRef)
        .collect();
    let mut chart_parts = parts(selecting(), values);
    let with = |high| {
        let mut selection = brush(proto::SelectionResolution::Global);
        selection.initial = vec![clause(high)];
        selection
    };
    chart_parts.0.selections = vec![with(1), with(1)];
    read(chart_parts.clone()).unwrap();
    chart_parts.0.selections = vec![with(1), with(2)];
    invalid(
        chart_parts,
        "selections[1].initial[0]",
        "differ, which is a conflict",
    );
}

#[test]
fn shared_fits_merge_field_by_field() {
    let fitted = |fit: proto::IntervalFit| {
        let mut layer = symbols(0);
        *data_fit(scale_of(&mut layer, false)) = Some(proto::DomainFit {
            fit: Some(proto::domain_fit::Fit::Interval(fit)),
        });
        plot(vec![layer])
    };
    let zero = proto::IntervalFit {
        zero: Some(true),
        ..Default::default()
    };
    let nice = proto::IntervalFit {
        nice: Some(5.0),
        ..Default::default()
    };
    let chart = read(parts(shared_row(vec![fitted(zero), fitted(nice)]), vec![])).unwrap();
    let groups = chart.resolve().scales;
    let mut x = groups
        .into_iter()
        .find(|group| group.channel == proto::Channel::X)
        .unwrap();
    assert_eq!(x.members.len(), 2);
    let merged = proto::IntervalFit {
        zero: Some(true),
        nice: Some(5.0),
        empty: None,
        domain_pin: None,
    };
    assert_eq!(
        *data_fit(&mut x.spec),
        Some(proto::DomainFit {
            fit: Some(proto::domain_fit::Fit::Interval(merged)),
        })
    );
    let other_zero = proto::IntervalFit {
        zero: Some(false),
        ..Default::default()
    };
    invalid(
        parts(shared_row(vec![fitted(zero), fitted(other_zero)]), vec![]),
        "root.concat.children[1].plot.cartesian.layers[0]",
        "fit",
    );
    // Fits that pin different places of the domain disagree.
    let pinned = |pin| proto::IntervalFit {
        domain_pin: Some(pin),
        ..Default::default()
    };
    let value = proto::ValueRef { index: 0 };
    let values = || -> Vec<ArrayRef> { vec![Arc::new(Float64Array::from(vec![0.0]))] };
    let low = pinned(proto::interval_fit::DomainPin::Low(value));
    let mid = pinned(proto::interval_fit::DomainPin::Mid(value));
    read(parts(shared_row(vec![fitted(low), fitted(nice)]), values())).unwrap();
    invalid(
        parts(shared_row(vec![fitted(low), fitted(mid)]), values()),
        "root.concat.children[1].plot.cartesian.layers[0]",
        "fit",
    );
}

#[test]
fn a_partition_column_of_every_layer_is_a_column_of_the_plot() {
    let mut node = selecting();
    plot_mut(&mut node).tools[0] = proto::Tool {
        kind: Some(proto::tool::Kind::Select(proto::Select {
            selection: "brush".to_string(),
            scope: vec!["sex".to_string()],
            form: Some(proto::select::Form::Interval(
                proto::IntervalSelect::default(),
            )),
        })),
    };
    if let Some(plot::Coord::Cartesian(cartesian)) = &mut plot_mut(&mut node).coord {
        cartesian.layers.push(symbols(0));
    }
    for index in 0..2 {
        plot_layer_mut(&mut node, index).partition = vec![value("sex", 0)];
    }
    let mut chart_parts = parts(node.clone(), strings(&["female", "male"]));
    chart_parts.0.selections = vec![brush(proto::SelectionResolution::Global)];
    read(chart_parts.clone()).unwrap();
    plot_layer_mut(&mut node, 1).partition = vec![value("sex", 1)];
    chart_parts.0.root = Some(node);
    invalid(
        chart_parts,
        "root.plot.tools[0].select.scope[0]",
        "not a partition column",
    );
}

#[test]
fn a_reader_ignores_the_sizes_of_plots_in_a_wrap() {
    let sized = |width: f32| {
        let mut node = plot(vec![symbols(0)]);
        plot_mut(&mut node).width = Some(width);
        plot_mut(&mut node).height = Some(width);
        if let Some(plot::Coord::Cartesian(cartesian)) = &mut plot_mut(&mut node).coord {
            cartesian.options = Some(proto::Cartesian {
                aspect_ratio: Some(1.5),
            });
        }
        node
    };
    let wrap = Node {
        node: Some(node::Node::Wrap(proto::Wrap {
            children: vec![sized(100.0), sized(200.0)],
            resolve: vec![proto::Resolve {
                key: Some(proto::resolve::Key::Channel(proto::Channel::X as i32)),
                sharing: proto::Sharing::Shared as i32,
            }],
            ..Default::default()
        })),
    };
    read(parts(wrap, vec![])).unwrap();
}

#[test]
fn tracks_of_one_size_match_in_any_spelling() {
    let tracked = |tracks: Vec<proto::Track>, children: Vec<Node>| {
        let mut row = shared_row(children);
        concat_mut(&mut row).layout = Some(proto::ConcatLayout {
            tracks,
            ..Default::default()
        });
        row
    };
    let share = |weight: Option<f32>| proto::Track {
        size: Some(track::Size::Share(proto::Share { weight, max: None })),
    };
    let length = |length: f32| proto::Track {
        size: Some(track::Size::Length(length)),
    };
    let two = || vec![plot(vec![symbols(0)]), plot(vec![symbols(0)])];
    read(parts(
        tracked(vec![share(Some(1.0)), share(None)], two()),
        vec![],
    ))
    .unwrap();
    invalid(
        parts(tracked(vec![length(100.0), share(None)], two()), vec![]),
        "root.concat",
        "sizes differ",
    );
    // A concat without children holds a track of its own.
    let mut three = two();
    three.insert(0, concat(proto::Direction::Column, vec![]));
    read(parts(
        tracked(vec![length(300.0), length(200.0), length(200.0)], three),
        vec![],
    ))
    .unwrap();
    invalid(
        parts(tracked(vec![share(None)], two()), vec![]),
        "root.concat.layout.tracks",
        "one track for each child",
    );
}

#[test]
fn a_dictionary_column_reads_as_its_values() {
    use arrow::{array::DictionaryArray, datatypes::Int32Type};
    let table = points();
    let labels: DictionaryArray<Int32Type> = vec!["p", "q", "p"].into_iter().collect();
    let mut columns = table.columns().to_vec();
    columns.push(Arc::new(labels));
    let mut fields: Vec<arrow::datatypes::Field> = table
        .schema()
        .fields()
        .iter()
        .map(|field| field.as_ref().clone())
        .collect();
    fields.push(arrow::datatypes::Field::new(
        "label",
        arrow::datatypes::DataType::Dictionary(
            Box::new(arrow::datatypes::DataType::Int32),
            Box::new(arrow::datatypes::DataType::Utf8),
        ),
        false,
    ));
    let table =
        RecordBatch::try_new(Arc::new(arrow::datatypes::Schema::new(fields)), columns).unwrap();
    let chart = read((chart(plot(vec![symbols(0)])), vec![table], vec![])).unwrap();
    assert_eq!(
        chart.tables()[0].column(2).data_type(),
        &arrow::datatypes::DataType::Utf8
    );
    assert_eq!(Chart::from_bytes(chart.to_bytes().unwrap()).unwrap(), chart);
}

#[test]
fn a_child_that_spans_several_tracks_sets_no_size() {
    let sized = |children: Vec<Node>, second: Option<track::Size>| {
        let mut row = concat(proto::Direction::Row, children);
        concat_mut(&mut row).layout = Some(proto::ConcatLayout {
            tracks: vec![proto::Track { size: None }, proto::Track { size: second }],
            ..Default::default()
        });
        row
    };
    let leaf = || plot(vec![symbols(0)]);
    let inner_row = || concat(proto::Direction::Row, vec![leaf(), leaf()]);
    // A row in a row places its two children in two column tracks of the outer row.
    read(parts(sized(vec![leaf(), inner_row()], None), vec![])).unwrap();
    invalid(
        parts(
            sized(vec![leaf(), inner_row()], Some(track::Size::Length(300.0))),
            vec![],
        ),
        "root.concat.layout.tracks[1]",
        "child 1 spans 2 columns",
    );
    // So does a column whose row has two children.
    let column = || concat(proto::Direction::Column, vec![inner_row(), leaf()]);
    invalid(
        parts(
            sized(vec![leaf(), column()], Some(track::Size::Length(300.0))),
            vec![],
        ),
        "root.concat.layout.tracks[1]",
        "child 1 spans 2 columns",
    );
    // A child in one track takes a size.
    read(parts(
        sized(vec![leaf(), leaf()], Some(track::Size::Length(300.0))),
        vec![],
    ))
    .unwrap();
}

#[test]
fn shared_scales_agree_on_their_kind_and_join_the_classes_of_their_guides() {
    let band = |kind: proto::ScaleKind, classes: &[&str]| {
        let mut layer = symbols(0);
        let spec = scale_of(&mut layer, false);
        spec.r#type = Some(scale::Type::Band(proto::BandScale {
            kind: kind as i32,
            ..Default::default()
        }));
        spec.domain = Some(proto::Domain {
            domain: Some(proto::domain::Domain::Data(proto::DataDomain::default())),
        });
        spec.guide = Some(proto::Guide {
            guide: Some(guide::Guide::Axis(proto::Axis {
                classes: classes.iter().map(|class| class.to_string()).collect(),
                ..Default::default()
            })),
        });
        layer
    };
    invalid(
        parts(
            plot(vec![
                band(proto::ScaleKind::Categorical, &[]),
                band(proto::ScaleKind::Ordinal, &[]),
            ]),
            vec![],
        ),
        "root.plot.cartesian.layers[1]",
        "kind",
    );

    let chart = read(parts(
        plot(vec![
            band(proto::ScaleKind::Unspecified, &["a"]),
            band(proto::ScaleKind::Ordinal, &["b", "a"]),
        ]),
        vec![],
    ))
    .unwrap();
    let groups = chart.resolve().scales;
    let x = (groups.iter())
        .find(|group| group.channel == proto::Channel::X)
        .unwrap();
    let Some(scale::Type::Band(band)) = &x.spec.r#type else {
        panic!("the shared scale is a band scale")
    };
    assert_eq!(band.kind, proto::ScaleKind::Ordinal as i32);
    let Some(guide::Guide::Axis(axis)) = x.spec.guide.as_ref().and_then(|g| g.guide.as_ref())
    else {
        panic!("the shared scale has an axis")
    };
    assert_eq!(axis.classes, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn shared_scales_agree_on_the_ids_of_their_guides() {
    let axis = |id: &str| {
        let mut layer = symbols(0);
        let spec = scale_of(&mut layer, false);
        spec.guide = Some(proto::Guide {
            guide: Some(guide::Guide::Axis(proto::Axis {
                id: id.to_string(),
                ..Default::default()
            })),
        });
        layer
    };
    invalid(
        parts(plot(vec![axis("time"), axis("date")]), vec![]),
        "root.plot.cartesian.layers[1]",
        "id",
    );

    let chart = read(parts(plot(vec![axis(""), axis("time")]), vec![])).unwrap();
    let groups = chart.resolve().scales;
    let x = (groups.iter())
        .find(|group| group.channel == proto::Channel::X)
        .unwrap();
    let Some(guide::Guide::Axis(axis)) = x.spec.guide.as_ref().and_then(|g| g.guide.as_ref())
    else {
        panic!("the shared scale has an axis")
    };
    assert_eq!(axis.id, "time");
}

/// Symbols whose `x` axis, or `y` axis when `y`, has `id`.
fn with_axis(mut layer: CartesianLayer, y: bool, id: &str) -> CartesianLayer {
    scale_of(&mut layer, y).guide = Some(proto::Guide {
        guide: Some(guide::Guide::Axis(proto::Axis {
            id: id.to_string(),
            ..Default::default()
        })),
    });
    layer
}

#[test]
fn ids_share_one_namespace_across_plots_layers_nodes_guides_and_headers() {
    let x_axis = "root.plot.cartesian.layers[0].mark.symbol.x.scale.guide.axis";
    let y_axis = "root.plot.cartesian.layers[0].mark.symbol.y.scale.guide.axis";
    read(parts(
        plot_with("sales", vec![], vec![with_axis(symbols(0), false, "time")]),
        vec![],
    ))
    .unwrap();

    // A layer and its plot.
    let mut layer = symbols(0);
    layer.id = "sales".to_string();
    invalid(
        parts(plot_with("sales", vec![], vec![layer]), vec![]),
        "root.plot.cartesian.layers[0]",
        "`root.plot` and `root.plot.cartesian.layers[0]` have one id",
    );

    // A guide and its plot, and two guides of one plot.
    invalid(
        parts(
            plot_with("sales", vec![], vec![with_axis(symbols(0), false, "sales")]),
            vec![],
        ),
        x_axis,
        "`root.plot` and",
    );
    let both = with_axis(with_axis(symbols(0), false, "axis"), true, "axis");
    invalid(
        parts(plot_with("", vec![], vec![both]), vec![]),
        y_axis,
        &format!("`{x_axis}` and `{y_axis}` have one id"),
    );

    // A facet header and its concat.
    let mut root = concat(
        proto::Direction::Row,
        vec![plot_with("", vec![value("species", 0)], vec![symbols(0)])],
    );
    let node = concat_mut(&mut root);
    node.id = "species".to_string();
    node.facet = Some(proto::Facet {
        column: "species".to_string(),
        header: Some(proto::facet::Header::Shown(proto::FacetHeader {
            id: "species".to_string(),
            ..Default::default()
        })),
    });
    invalid(
        parts(root, strings(&["Adelie"])),
        "root.concat.facet.shown",
        "`root.concat` and `root.concat.facet.shown` have one id",
    );
}

#[test]
fn the_guides_of_the_facets_of_independent_scales_differ_by_partition() {
    let facet = |sharing: proto::Sharing| {
        let child = |index| {
            plot_with(
                "",
                vec![value("species", index)],
                vec![with_axis(symbols(0), true, "measure")],
            )
        };
        let mut root = concat(proto::Direction::Row, vec![child(0), child(1)]);
        let node = concat_mut(&mut root);
        node.facet = Some(proto::Facet {
            column: "species".to_string(),
            header: None,
        });
        node.resolve = vec![proto::Resolve {
            key: Some(proto::resolve::Key::Channel(proto::Channel::Y as i32)),
            sharing: sharing as i32,
        }];
        parts(root, strings(&["Adelie", "Gentoo"]))
    };
    // A shared `y` has one axis, and independent ones an axis for each facet's partition.
    for sharing in [proto::Sharing::Shared, proto::Sharing::Independent] {
        let chart = read(facet(sharing)).unwrap();
        let resolved = chart.resolve();
        let identities: Vec<String> = (resolved.scales.iter())
            .filter_map(|group| group.guide_identity(&resolved.tree))
            .map(|identity| identity.name())
            .collect();
        let expected: &[&str] = match sharing {
            proto::Sharing::Shared => &["measure"],
            _ => &["measure[species=Adelie]", "measure[species=Gentoo]"],
        };
        assert_eq!(identities, expected);
    }

    // Plots without partitions and with independent scales give their axes one identity.
    let plain = concat(
        proto::Direction::Row,
        vec![
            plot(vec![with_axis(symbols(0), true, "measure")]),
            plot(vec![with_axis(symbols(0), true, "measure")]),
        ],
    );
    invalid(
        parts(plain, vec![]),
        "root.concat.children[1].plot.cartesian.layers[0].mark.symbol.y.scale.guide.axis",
        "have one id and one partition, `measure`",
    );
}

#[test]
fn an_id_holds_no_nul_character() {
    invalid(
        parts(plot_with("a\0b", vec![], vec![symbols(0)]), vec![]),
        "root.plot",
        "no NUL character",
    );
    assert_eq!(
        avenger_chart::resolve::Identity::owner_name("", "scatter"),
        "scatter"
    );
    assert_eq!(
        avenger_chart::resolve::Identity::owner_name("magnitude", "scatter"),
        "\0magnitude\0scatter"
    );
}
