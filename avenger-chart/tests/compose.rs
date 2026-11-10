//! The composition API of the builders: a builder tree's own data, finished charts as nodes, the
//! overlay of plots, and partition values, from a table or one by one.
//!
//! Each test writes its charts in full with the builders, as a producer writes them.

mod support;

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, AsArray, DictionaryArray, Float64Array, RecordBatch, StringArray},
    datatypes::{DataType, Float64Type, Int32Type},
    ipc::{writer::IpcWriteOptions, MetadataVersion},
};
use avenger_chart::{
    prelude::*,
    proto::{cartesian_layer, domain, node, plot::Coord, Table},
};
use support::{chart, ipc, points, symbols};

/// Fails unless `result` is invalid at `path`, for a reason that contains `reason`.
#[track_caller]
fn fails<T: std::fmt::Debug>(result: Result<T, ChartError>, path: &str, reason: &str) {
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

fn root_plot(chart: &Chart) -> &Plot {
    match &chart.root().node {
        Some(node::Node::Plot(plot)) => plot,
        _ => panic!("the root is a plot"),
    }
}

fn layers(plot: &Plot) -> &[CartesianLayer] {
    match &plot.coord {
        Some(Coord::Cartesian(cartesian)) => &cartesian.layers,
        _ => panic!("the plot is Cartesian"),
    }
}

#[test]
fn a_builder_tree_refers_to_its_own_tables_and_values() {
    let symbols = CartesianLayer::symbol(
        points(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("a")
                .scale(Scale::linear(LinearScale::builder()).domain(Domain::explicit([0.0, 10.0]))))
            .y("b"),
    )
    .partition([PartitionValue::new("sex", "female")]);
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

    let Some(cartesian_layer::Mark::Symbol(mark)) = &layers(root_plot(&chart))[0].mark else {
        panic!("the layer draws symbols")
    };
    let x = mark.x.as_ref().unwrap().scale.as_ref().unwrap();
    let Some(domain::Domain::Explicit(domain)) = x.domain.as_ref().unwrap().domain else {
        panic!("the domain is explicit")
    };
    let ends = chart.list_elements(&domain);
    assert_eq!(ends.as_primitive::<Float64Type>().values(), &[0.0, 10.0]);
    assert!(chart.resolve().tree.partitions["root.plot"].contains_key("sex"));
}

#[test]
fn nested_builders_refer_to_their_own_values() {
    // Each layer has its own `y` and its own explicit domain, after the values of the layers
    // before it.
    let layer = |low: f64| {
        CartesianLayer::symbol(
            points(),
            SymbolMark::builder()
                .x("a")
                .y(Encoding::scaled_field("b").scale(
                    Scale::linear(LinearScale::builder())
                        .domain(Domain::explicit([low, low + 10.0])),
                )),
        )
    };
    let chart = Chart::builder(
        Plot::cartesian([layer(0.0), layer(5.0)])
            .resolve([Resolve::channel(Sharing::Independent, Channel::Y)]),
    )
    .build()
    .unwrap();

    let lows: Vec<f64> = (chart.resolve().scales.iter())
        .filter(|group| group.channel == Channel::Y)
        .map(|group| {
            let Some(domain::Domain::Explicit(value)) = group.spec.domain.as_ref().unwrap().domain
            else {
                panic!("the domain is explicit")
            };
            chart
                .list_elements(&value)
                .as_primitive::<Float64Type>()
                .value(0)
        })
        .collect();
    assert_eq!(lows, [0.0, 5.0]);
}

#[test]
fn a_concat_of_finished_charts_copies_their_streams() {
    // Charts read from messages whose tables are IPC streams, as another producer writes them.
    let options = IpcWriteOptions::try_new(8, false, MetadataVersion::V5).unwrap();
    let stream = ipc(&points(), options.clone());
    let read = || {
        let mut message = chart(support::plot(vec![symbols(0)]));
        message.tables = vec![Table {
            padding: Default::default(),
            ipc: ipc(&points(), options.clone()).into(),
        }];
        Chart::from_message(message).unwrap()
    };
    let combined = Chart::builder(Concat::new(Direction::Row).children([read(), read()]))
        .build()
        .unwrap();

    // Two copies of one table are one table, written as its stream.
    assert_eq!(combined.tables().len(), 1);
    let bytes = combined.to_bytes().unwrap();
    assert!(bytes
        .windows(stream.len())
        .any(|window| window == stream.as_slice()));
}

#[test]
fn format_syntaxes_agree_and_outer_sizes_drop() {
    let scatter = |syntax: &str, width: Option<f32>| {
        let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"));
        let chart = Chart::builder(Plot::cartesian([symbols])).number_format_syntax(syntax);
        match width {
            Some(width) => chart.width(width),
            None => chart,
        }
        .build()
        .unwrap()
    };

    let combined = Chart::builder(
        Concat::new(Direction::Row).children([scatter("d3", Some(300.0)), scatter("", None)]),
    )
    .build()
    .unwrap();
    assert_eq!(combined.message().number_format_syntax, "d3");
    assert_eq!(combined.message().width, None);

    fails(
        Chart::builder(
            Concat::new(Direction::Row).children([scatter("d3", None), scatter("icu", None)]),
        )
        .build(),
        "root.concat.children[1]",
        "which is a conflict",
    );
    // The builder's own syntax takes part in the agreement.
    fails(
        Chart::builder(Concat::new(Direction::Row).children([scatter("d3", None)]))
            .number_format_syntax("icu")
            .build(),
        "root.concat.children[0]",
        "which is a conflict",
    );
}

#[test]
fn the_selections_of_finished_charts_join() {
    let selecting = |id: &str| {
        let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"));
        let plot = Plot::cartesian([symbols])
            .id(id)
            .tools([Select::interval("brush", IntervalSelect::builder())]);
        Chart::builder(plot)
            .selections([Selection::new("brush", SelectionResolution::Global)])
            .build()
            .unwrap()
    };
    let combined =
        Chart::builder(Concat::new(Direction::Row).children([selecting("a"), selecting("b")]))
            .build()
            .unwrap();
    assert_eq!(combined.selections().len(), 1);
}

#[test]
fn a_wrapped_facet_keeps_the_partitions_of_its_children() {
    let panel = |genre: &str| {
        let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"));
        Chart::builder(
            Plot::cartesian([symbols])
                .id("panel")
                .partition([PartitionValue::new("genre", genre)]),
        )
        .build()
        .unwrap()
    };
    let chart = Chart::builder(
        Wrap::builder()
            .facet(Facet::builder().column("genre"))
            .children([panel("Drama"), panel("Comedy")]),
    )
    .build()
    .unwrap();

    let resolved = chart.resolve();
    for index in 0..2 {
        let path = format!("root.wrap.children[{index}].plot");
        assert!(resolved.tree.partitions[&path].contains_key("genre"));
    }
}

#[test]
fn a_facet_names_its_column() {
    let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"));
    let panel = Plot::cartesian([symbols]).partition([PartitionValue::new("genre", "Drama")]);
    let chart = Chart::builder(Wrap::builder().facet(Facet::builder()).children([panel])).build();
    fails(
        chart,
        "root.wrap.facet.column",
        "`column` is required and not empty",
    );
}

#[test]
fn a_finished_chart_takes_a_partition_value() {
    let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"));
    let scatter = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let chart = Chart::builder(
        NodeBuilder::from(scatter).partition([PartitionValue::new("sex", "female")]),
    )
    .build()
    .unwrap();

    let partition = &chart.resolve().tree.partitions["root.plot"];
    assert_eq!(partition["sex"].as_string::<i32>().value(0), "female");
    assert_eq!(partition["sex"].len(), 1);

    fails(
        Chart::builder(NodeBuilder::default().partition([PartitionValue::new("sex", "female")]))
            .build(),
        "root",
        "a node without a member",
    );
}

/// A table of the key column `keys`, and the row numbers `row`.
fn keyed(keys: ArrayRef) -> RecordBatch {
    let rows: Vec<f64> = (0..keys.len()).map(|row| row as f64).collect();
    RecordBatch::try_from_iter([
        ("key", keys),
        ("row", Arc::new(Float64Array::from(rows)) as ArrayRef),
    ])
    .unwrap()
}

/// The value of each node of `nodes`, in order, and the rows of its table, in a facet of `key`.
fn cells(nodes: Vec<NodeBuilder>) -> Vec<(String, Vec<f64>)> {
    let chart = Chart::builder(
        Concat::new(Direction::Row)
            .facet(Facet::builder().column("key"))
            .children(nodes),
    )
    .build()
    .unwrap();
    let Some(node::Node::Concat(concat)) = &chart.root().node else {
        panic!("the root is a concat")
    };
    let resolved = chart.resolve();
    (concat.children.iter().enumerate())
        .map(|(index, child)| {
            let partition =
                &resolved.tree.partitions[&format!("root.concat.children[{index}].plot")];
            let key = &partition["key"];
            let value = match (key.is_null(0), key.data_type()) {
                (true, _) => "null".to_string(),
                (false, DataType::Float64) => {
                    key.as_primitive::<Float64Type>().value(0).to_string()
                }
                (false, _) => key.as_string::<i32>().value(0).to_string(),
            };
            let Some(node::Node::Plot(plot)) = &child.node else {
                panic!("each cell is a plot")
            };
            let table = layers(plot)[0].table.unwrap();
            let rows = chart.tables()[table.index as usize]
                .column_by_name("row")
                .unwrap()
                .as_primitive::<Float64Type>()
                .values()
                .to_vec();
            (value, rows)
        })
        .collect()
}

#[test]
fn partitions_split_a_table_in_each_order() {
    let table = keyed(Arc::new(StringArray::from(vec![
        Some("b"),
        None,
        Some("a"),
        Some("b"),
        Some("c"),
    ])));
    let split = |order| {
        let nodes = Node::partitions(&table, "key", order, |rows| {
            let symbols = CartesianLayer::symbol(rows, SymbolMark::builder().x("row").y("row"));
            Ok(Plot::cartesian([symbols]))
        })
        .unwrap();
        cells(nodes)
    };

    // The rows of each part keep their order, and a null key forms its own part.
    assert_eq!(
        split(PartitionOrder::Appearance),
        [
            ("b".to_string(), vec![0.0, 3.0]),
            ("null".to_string(), vec![1.0]),
            ("a".to_string(), vec![2.0]),
            ("c".to_string(), vec![4.0]),
        ]
    );
    let names = |cells: Vec<(String, Vec<f64>)>| -> Vec<String> {
        cells.into_iter().map(|(name, _)| name).collect()
    };
    // Nulls come last in both sorted orders.
    assert_eq!(
        names(split(PartitionOrder::Ascending)),
        ["a", "b", "c", "null"]
    );
    assert_eq!(
        names(split(PartitionOrder::Descending)),
        ["c", "b", "a", "null"]
    );
}

#[test]
fn partitions_of_numbers_sort_nan_above_every_number() {
    let table = keyed(Arc::new(Float64Array::from(vec![2.0, f64::NAN, 1.0])));
    let split = |order| {
        let nodes = Node::partitions(&table, "key", order, |rows| {
            let symbols = CartesianLayer::symbol(rows, SymbolMark::builder().x("row").y("row"));
            Ok(Plot::cartesian([symbols]))
        })
        .unwrap();
        (cells(nodes).into_iter())
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
    };
    assert_eq!(split(PartitionOrder::Ascending), ["1", "2", "NaN"]);
    assert_eq!(split(PartitionOrder::Descending), ["NaN", "2", "1"]);
}

#[test]
fn partitions_of_a_dictionary_take_its_values() {
    let keys: DictionaryArray<Int32Type> = vec!["x", "y", "x"].into_iter().collect();
    let nodes = Node::partitions(
        &keyed(Arc::new(keys)),
        "key",
        PartitionOrder::Ascending,
        |rows| {
            let symbols = CartesianLayer::symbol(rows, SymbolMark::builder().x("row").y("row"));
            Ok(Plot::cartesian([symbols]))
        },
    )
    .unwrap();
    assert_eq!(
        cells(nodes),
        [
            ("x".to_string(), vec![0.0, 2.0]),
            ("y".to_string(), vec![1.0])
        ]
    );
}

#[test]
fn partitions_of_a_missing_column_fail() {
    let table = keyed(Arc::new(StringArray::from(vec!["a"])));
    fails(
        Node::partitions(&table, "missing", PartitionOrder::Appearance, |_| {
            Ok(Plot::overlay(Vec::<NodeBuilder>::new()))
        }),
        "missing",
        "no column `missing`",
    );
}

#[test]
fn an_overlay_draws_the_layers_of_its_plots_in_one_plot() {
    // Two plots of one table, each over its own part of a partition, the first with a title.
    let female = Plot::cartesian([CartesianLayer::symbol(
        points(),
        SymbolMark::builder().x("a").y("b"),
    )])
    .title(Title::new("Mass"))
    .partition([PartitionValue::new("sex", "female")]);
    let male = Plot::cartesian([CartesianLayer::symbol(
        points(),
        SymbolMark::builder().x("a").y("b"),
    )])
    .partition([PartitionValue::new("sex", "male")]);
    let chart = Chart::builder(Plot::overlay([female, male]))
        .build()
        .unwrap();

    let plot = root_plot(&chart);
    assert_eq!(plot.title.as_ref().unwrap().text, "Mass");
    assert_eq!(layers(plot).len(), 2);
    // The plots' partitions differ, so each layer declares its own.
    assert!(plot.partition.is_empty());
    assert!(layers(plot).iter().all(|layer| layer.partition.len() == 1));
}

#[test]
fn the_fields_of_an_overlay_plot_win_and_its_layers_come_first() {
    let titled = |title: &str| {
        Plot::cartesian([CartesianLayer::symbol(
            points(),
            SymbolMark::builder().x("a").y("b"),
        )])
        .title(Title::new(title))
    };
    let own = Plot::cartesian([CartesianLayer::rule(points(), RuleMark::builder().y(5.0))])
        .title(Title::new("Both"))
        .overlay([titled("A"), titled("B")]);
    let chart = Chart::builder(own).build().unwrap();

    let plot = root_plot(&chart);
    assert_eq!(plot.title.as_ref().unwrap().text, "Both");
    let marks: Vec<&str> = (layers(plot).iter())
        .map(|layer| match layer.mark {
            Some(cartesian_layer::Mark::Rule(_)) => "rule",
            Some(cartesian_layer::Mark::Symbol(_)) => "symbol",
            _ => "other",
        })
        .collect();
    assert_eq!(marks, ["rule", "symbol", "symbol"]);

    // Without a title of its own, the overlaid plots' titles must agree.
    fails(
        Chart::builder(Plot::overlay([titled("A"), titled("B")])).build(),
        "root.plot.overlay[1]",
        "which is a conflict",
    );
}

#[test]
fn the_options_of_overlaid_plots_agree() {
    let shaped = |ratio: f32| {
        Plot::cartesian(
            CartesianPlot::new([CartesianLayer::symbol(
                points(),
                SymbolMark::builder().x("a").y("b"),
            )])
            .options(Cartesian::builder().aspect_ratio(ratio)),
        )
    };
    Chart::builder(Plot::overlay([shaped(1.5), shaped(1.5)]))
        .build()
        .unwrap();
    fails(
        Chart::builder(Plot::overlay([shaped(1.5), shaped(2.0)])).build(),
        "root.plot.overlay[1]",
        "`options`",
    );
}

#[test]
fn an_overlaid_plot_is_a_plot_of_the_coordinate_system() {
    let cartesian = Plot::cartesian([CartesianLayer::symbol(
        points(),
        SymbolMark::builder().x("a").y("b"),
    )]);
    let polar = Plot::polar([PolarLayer::symbol(
        points(),
        PolarSymbolMark::builder().theta("a").radius("b"),
    )]);
    fails(
        Chart::builder(Plot::overlay([cartesian.clone(), polar])).build(),
        "root.plot.overlay[1]",
        "the coordinate system of the plot",
    );

    let row = Concat::new(Direction::Row).children([cartesian]);
    fails(
        Chart::builder(Plot::overlay([row])).build(),
        "root.plot.overlay[0]",
        "an overlaid node is a plot",
    );
}

#[test]
fn an_overlay_joins_the_classes_of_its_plots() {
    let classed = |classes: &[&str]| {
        Plot::cartesian([CartesianLayer::symbol(
            points(),
            SymbolMark::builder().x("a").y("b"),
        )])
        .classes(classes.iter().copied())
    };
    let chart = Chart::builder(
        Plot::overlay([classed(&["a"]), classed(&["report", "b"])]).classes(["report"]),
    )
    .build()
    .unwrap();
    assert_eq!(root_plot(&chart).classes, ["report", "a", "b"]);
}

/// Charts built apart that each declare one filter combine into a chart with one widget.
#[test]
fn combined_charts_share_the_widgets_that_they_declare() {
    // A scatter plot whose layer declares a magnitude filter over `a`, which writes `filters`.
    let filtered = || {
        let magnitude = Widget::range_slider(
            "magnitude",
            RangeSliderWidget::new(
                WidgetTarget::new("filters", "a"),
                WidgetDataDomain::builder(),
            ),
        );
        let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"))
            .widgets([magnitude]);
        Chart::builder(Plot::cartesian([symbols]))
            .selections([Selection::new("filters", SelectionResolution::Intersect)])
            .build()
            .unwrap()
    };
    let overlaid = Chart::builder(Plot::overlay([filtered(), filtered()]))
        .build()
        .unwrap();
    let side_by_side =
        Chart::builder(Concat::new(Direction::Row).children([filtered(), filtered()]))
            .build()
            .unwrap();
    for combined in [overlaid, side_by_side] {
        let resolved = combined.resolve();
        assert_eq!(resolved.widgets.len(), 1);
        assert_eq!(resolved.widgets[0].members.len(), 2);
        assert!(resolved.widgets[0].anchor.is_none());
    }
}

#[test]
fn a_wrapped_facet_of_partitions() {
    let cars = RecordBatch::try_from_iter([
        (
            "origin",
            Arc::new(StringArray::from(vec!["USA", "Japan", "USA", "Europe"])) as ArrayRef,
        ),
        (
            "weight",
            Arc::new(Float64Array::from(vec![3.5, 2.2, 4.1, 2.8])) as ArrayRef,
        ),
        (
            "mpg",
            Arc::new(Float64Array::from(vec![18.0, 31.0, 15.0, 26.0])) as ArrayRef,
        ),
    ])
    .unwrap();

    // One panel of `mpg` against `weight` for each origin, in a wrap that facets the origins.
    let panels = Node::partitions(&cars, "origin", PartitionOrder::Ascending, |rows| {
        let symbols = CartesianLayer::symbol(rows, SymbolMark::builder().x("weight").y("mpg"));
        Ok(Plot::cartesian([symbols]).id("panel"))
    })
    .unwrap();
    let chart = Chart::builder(
        Wrap::builder()
            .facet(Facet::builder().column("origin"))
            .children(panels),
    )
    .build()
    .unwrap();

    let Some(node::Node::Wrap(wrap)) = &chart.root().node else {
        panic!("the root is a wrap")
    };
    let resolved = chart.resolve();
    let expected = [
        ("Europe", vec![2.8]),
        ("Japan", vec![2.2]),
        ("USA", vec![3.5, 4.1]),
    ];
    assert_eq!(wrap.children.len(), expected.len());
    for (index, (origin, weights)) in expected.iter().enumerate() {
        let partition = &resolved.tree.partitions[&format!("root.wrap.children[{index}].plot")];
        assert_eq!(partition["origin"].as_string::<i32>().value(0), *origin);
        let Some(node::Node::Plot(plot)) = &wrap.children[index].node else {
            panic!("each panel is a plot")
        };
        let table = &chart.tables()[layers(plot)[0].table.unwrap().index as usize];
        let column = table.column_by_name("weight").unwrap();
        assert_eq!(
            column.as_primitive::<Float64Type>().values(),
            weights.as_slice()
        );
    }
}

#[test]
fn a_facet_grid_of_nested_partitions() {
    // West has no store sales, so its row has no store cell.
    let sales = RecordBatch::try_from_iter([
        (
            "region",
            Arc::new(StringArray::from(vec!["East", "East", "West"])) as ArrayRef,
        ),
        (
            "channel",
            Arc::new(StringArray::from(vec!["web", "store", "web"])) as ArrayRef,
        ),
        (
            "amount",
            Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])) as ArrayRef,
        ),
    ])
    .unwrap();

    // A column that facets the regions, of rows that facet the channels, of a bar for each cell.
    let rows = Node::partitions(&sales, "region", PartitionOrder::Ascending, |region| {
        let cells = Node::partitions(&region, "channel", PartitionOrder::Ascending, |cell| {
            let bars = CartesianLayer::bar(cell, BarMark::builder().x("channel").y("amount"));
            Ok(Plot::cartesian([bars]))
        })?;
        Ok(Concat::new(Direction::Row)
            .facet(Facet::builder().column("channel"))
            .children(cells))
    })
    .unwrap();
    let chart = Chart::builder(
        Concat::new(Direction::Column)
            .facet(Facet::builder().column("region"))
            .children(rows),
    )
    .build()
    .unwrap();

    let Some(node::Node::Concat(column)) = &chart.root().node else {
        panic!("the root is a concat")
    };
    let resolved = chart.resolve();
    let value = |path: &str, key: &str| {
        resolved.tree.partitions[path][key]
            .as_string::<i32>()
            .value(0)
            .to_string()
    };
    let grid: Vec<(String, Vec<String>)> = (column.children.iter().enumerate())
        .map(|(row, child)| {
            let Some(node::Node::Concat(cells)) = &child.node else {
                panic!("each row is a concat")
            };
            let row_path = format!("root.concat.children[{row}].concat");
            let channels = (0..cells.children.len())
                .map(|cell| value(&format!("{row_path}.children[{cell}].plot"), "channel"))
                .collect();
            (value(&row_path, "region"), channels)
        })
        .collect();
    assert_eq!(
        grid,
        [
            (
                "East".to_string(),
                vec!["store".to_string(), "web".to_string()]
            ),
            ("West".to_string(), vec!["web".to_string()]),
        ]
    );
}
