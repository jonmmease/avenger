//! The composition API of the builders: a builder tree's own data, finished charts as nodes, the
//! overlay of plots, and partition values, from a table or one by one.

mod support;

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, AsArray, DictionaryArray, Float64Array, RecordBatch, StringArray},
    datatypes::{Float64Type, Int32Type},
    ipc::{writer::IpcWriteOptions, MetadataVersion},
};
use avenger_chart::{
    prelude::*,
    proto::{cartesian_layer, domain, node, plot::Coord, Table},
};
use support::*;

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

/// Symbols of `a` against `b` over `table`.
fn layer(table: RecordBatch) -> CartesianLayerBuilder {
    CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_column("a"))
            .y(Encoding::scaled_column("b")),
    )
    .table(table)
}

/// A scatter plot with `id`, whose plot declares `partition`.
fn plot(id: &str, partition: &[(&str, &str)]) -> PlotBuilder {
    Plot::cartesian(CartesianPlot::builder().layers([layer(points())]))
        .id(id)
        .partition(
            (partition.iter())
                .map(|(column, value)| PartitionValue::builder().column(*column).value(*value)),
        )
}

/// The chart of [`plot`].
fn scatter(id: &str, partition: &[(&str, &str)]) -> Chart {
    Chart::builder()
        .root(plot(id, partition))
        .infer_scales()
        .build()
        .unwrap()
}

/// The chart of `root`, with its scales inferred.
fn chart_of(root: impl Into<NodeBuilder>) -> Result<Chart, ChartError> {
    Chart::builder().root(root).infer_scales().build()
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

fn row() -> ConcatBuilder {
    Concat::builder().direction(Direction::Row)
}

#[test]
fn a_builder_tree_refers_to_its_own_tables_and_values() {
    let x = Encoding::scaled_column("a")
        .scale(ScaleSpec::linear(LinearScale::builder()).domain(Domain::explicit([0.0, 10.0])));
    let symbols =
        CartesianLayer::symbol(SymbolMark::builder().x(x).y(Encoding::scaled_column("b")))
            .table(points())
            .partition([PartitionValue::builder().column("sex").value("female")]);
    let chart = chart_of(Plot::cartesian(CartesianPlot::builder().layers([symbols]))).unwrap();
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
    // Each layer has its own `y` and its own explicit domain, after a value of the first layer.
    let layer = |low: f64| {
        let y = Encoding::scaled_column("b").scale(
            ScaleSpec::linear(LinearScale::builder()).domain(Domain::explicit([low, low + 10.0])),
        );
        CartesianLayer::symbol(SymbolMark::builder().x(Encoding::scaled_column("a")).y(y))
            .table(points())
    };
    let plot = Plot::cartesian(CartesianPlot::builder().layers([layer(0.0), layer(5.0)]))
        .resolve([Resolve::channel(Channel::Y).sharing(Sharing::Independent)]);
    let chart = chart_of(plot).unwrap();
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
    let combined = chart_of(row().children([read(), read()])).unwrap();
    // Two copies of one table are one table, written as its stream.
    assert_eq!(combined.tables().len(), 1);
    let bytes = combined.to_bytes().unwrap();
    assert!(bytes
        .windows(stream.len())
        .any(|window| window == stream.as_slice()));
}

#[test]
fn format_syntaxes_agree_and_outer_sizes_drop() {
    let with = |syntax: &str, width: Option<f32>| {
        let (mut message, tables, values) = scatter("", &[]).into_parts();
        message.number_format_syntax = syntax.to_string();
        message.width = width;
        Chart::from_parts(message, tables, values).unwrap()
    };
    let combined = chart_of(row().children([with("d3", Some(300.0)), with("", None)])).unwrap();
    assert_eq!(combined.message().number_format_syntax, "d3");
    assert_eq!(combined.message().width, None);
    fails(
        chart_of(row().children([with("d3", None), with("icu", None)])),
        "root.concat.children[1]",
        "which is a conflict",
    );
    // The builder's own syntax takes part in the agreement.
    fails(
        Chart::builder()
            .number_format_syntax("icu")
            .root(row().children([with("d3", None)]))
            .build(),
        "root.concat.children[0]",
        "which is a conflict",
    );
}

#[test]
fn the_selections_of_finished_charts_join() {
    let selecting = |id: &str| {
        let mut node = selecting();
        plot_mut(&mut node).id = id.to_string();
        let mut message = chart(node);
        message.selections = vec![brush(SelectionResolution::Global)];
        Chart::from_parts(message, vec![points()], vec![]).unwrap()
    };
    let combined = chart_of(row().children([selecting("a"), selecting("b")])).unwrap();
    assert_eq!(combined.selections().len(), 1);
}

#[test]
fn a_wrapped_facet_keeps_the_partitions_of_its_children() {
    let wrap = Wrap::builder()
        .facet(Facet::builder().column("genre"))
        .children([
            scatter("panel", &[("genre", "Drama")]),
            scatter("panel", &[("genre", "Comedy")]),
        ]);
    let chart = chart_of(wrap).unwrap();
    let resolved = chart.resolve();
    for index in 0..2 {
        let path = format!("root.wrap.children[{index}].plot");
        assert!(resolved.tree.partitions[&path].contains_key("genre"));
    }
}

#[test]
fn a_finished_chart_takes_a_partition_value() {
    let cell = NodeBuilder::from(scatter("", &[]))
        .partition([PartitionValue::builder().column("sex").value("female")]);
    let chart = chart_of(cell).unwrap();
    let partition = &chart.resolve().tree.partitions["root.plot"];
    assert_eq!(partition["sex"].as_string::<i32>().value(0), "female");
    assert_eq!(partition["sex"].len(), 1);
    fails(
        chart_of(NodeBuilder::default().partition([PartitionValue::builder().column("sex")])),
        "root",
        "a node without a member",
    );
}

/// A table of a key column `key`, with a null and repeats, and the row numbers `row`.
fn keyed(keys: ArrayRef) -> RecordBatch {
    let rows: Vec<f64> = (0..keys.len()).map(|row| row as f64).collect();
    RecordBatch::try_from_iter([
        ("key", keys),
        ("row", Arc::new(Float64Array::from(rows)) as ArrayRef),
    ])
    .unwrap()
}

/// The value of each node of `nodes` and the rows of its table.
fn cells(nodes: Vec<NodeBuilder>) -> Vec<(String, Vec<f64>)> {
    let concat = chart_of(row().facet(Facet::builder().column("key")).children(nodes)).unwrap();
    let Some(node::Node::Concat(concat_node)) = &concat.root().node else {
        panic!("the root is a concat")
    };
    let resolved = concat.resolve();
    (0..concat_node.children.len())
        .map(|index| {
            let partition =
                &resolved.tree.partitions[&format!("root.concat.children[{index}].plot")];
            let key = &partition["key"];
            let value = match key.is_null(0) {
                true => "null".to_string(),
                false => match key.data_type() {
                    arrow::datatypes::DataType::Float64 => {
                        key.as_primitive::<Float64Type>().value(0).to_string()
                    }
                    _ => key.as_string::<i32>().value(0).to_string(),
                },
            };
            let Some(node::Node::Plot(plot)) = &concat_node.children[index].node else {
                panic!("each cell is a plot")
            };
            let table = layers(plot)[0].table.unwrap();
            let rows = concat.tables()[table.index as usize]
                .column_by_name("row")
                .unwrap()
                .as_primitive::<Float64Type>()
                .values()
                .to_vec();
            (value, rows)
        })
        .collect()
}

/// A plot of the rows of a part, with `row` on `x` and `y`.
fn part(rows: RecordBatch) -> Result<PlotBuilder, ChartError> {
    Ok(Plot::cartesian(
        CartesianPlot::builder().layers([CartesianLayer::symbol(
            SymbolMark::builder()
                .x(Encoding::scaled_column("row"))
                .y(Encoding::scaled_column("row")),
        )
        .table(rows)]),
    ))
}

#[test]
fn partitions_split_a_table_in_each_order() {
    let keys = || {
        Arc::new(StringArray::from(vec![
            Some("b"),
            None,
            Some("a"),
            Some("b"),
            Some("c"),
        ])) as ArrayRef
    };
    let split = |order| cells(Node::partitions(&keyed(keys()), "key", order, part).unwrap());
    let appearance = split(PartitionOrder::Appearance);
    assert_eq!(
        appearance,
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
    let keys = Arc::new(Float64Array::from(vec![2.0, f64::NAN, 1.0])) as ArrayRef;
    let split = |order| {
        let cells = cells(Node::partitions(&keyed(keys.clone()), "key", order, part).unwrap());
        cells.into_iter().map(|(name, _)| name).collect::<Vec<_>>()
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
        part,
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
    let keys = Arc::new(StringArray::from(vec!["a"])) as ArrayRef;
    fails(
        Node::partitions(&keyed(keys), "missing", PartitionOrder::Appearance, part),
        "missing",
        "no column `missing`",
    );
}

/// The title of a plot.
fn title(text: &str) -> TitleBuilder {
    Title::builder().text(text)
}

#[test]
fn an_overlay_draws_the_layers_of_its_plots_in_one_plot() {
    let overlaid = Plot::builder().overlay([
        NodeBuilder::from(plot("", &[("sex", "female")]).title(title("Mass"))),
        NodeBuilder::from(plot("", &[("sex", "male")])),
    ]);
    let chart = chart_of(overlaid).unwrap();
    let plot = root_plot(&chart);
    assert_eq!(plot.title.as_ref().unwrap().text, "Mass");
    assert_eq!(layers(plot).len(), 2);
    // The plots' partitions differ, so each layer declares its own.
    assert!(plot.partition.is_empty());
    assert!(layers(plot).iter().all(|layer| layer.partition.len() == 1));

    // The plot's own fields win, and its own layers come first.
    let own = Plot::cartesian(CartesianPlot::builder().layers([layer(points())]))
        .title(title("Both"))
        .overlay([scatter("", &[]), chart_of(titled_plot("B")).unwrap()]);
    let chart = chart_of(own).unwrap();
    let plot = root_plot(&chart);
    assert_eq!(plot.title.as_ref().unwrap().text, "Both");
    assert_eq!(layers(plot).len(), 3);

    fails(
        chart_of(Plot::builder().overlay([titled_plot("A"), titled_plot("B")])),
        "root.plot.overlay[1]",
        "which is a conflict",
    );
}

/// A scatter plot titled `text`.
fn titled_plot(text: &str) -> PlotBuilder {
    plot("", &[]).title(title(text))
}

#[test]
fn the_options_of_overlaid_plots_agree() {
    let shaped = |ratio: f32| {
        Plot::cartesian(
            CartesianPlot::builder()
                .layers([layer(points())])
                .options(Cartesian::builder().aspect_ratio(ratio)),
        )
    };
    chart_of(Plot::builder().overlay([shaped(1.5), shaped(1.5)])).unwrap();
    fails(
        chart_of(Plot::builder().overlay([shaped(1.5), shaped(2.0)])),
        "root.plot.overlay[1]",
        "`options`",
    );
}

#[test]
fn an_overlaid_plot_is_a_plot_of_the_coordinate_system() {
    let polar = Plot::polar(
        PolarPlot::builder().layers([PolarLayer::symbol(
            PolarSymbolMark::builder()
                .theta(Encoding::scaled_column("a"))
                .radius(Encoding::scaled_column("b")),
        )
        .table(points())]),
    );
    fails(
        chart_of(Plot::builder().overlay([NodeBuilder::from(plot("", &[])), polar.into()])),
        "root.plot.overlay[1]",
        "the coordinate system of the plot",
    );
    fails(
        chart_of(Plot::builder().overlay([row().children([scatter("", &[])])])),
        "root.plot.overlay[0]",
        "an overlaid node is a plot",
    );
}

#[test]
fn an_overlay_joins_the_classes_of_its_plots() {
    let classed = |classes: &[&str]| plot("", &[]).classes(classes.iter().copied());
    let overlaid = Plot::builder()
        .classes(["report"])
        .overlay([classed(&["a"]), classed(&["report", "b"])]);
    let chart = chart_of(overlaid).unwrap();
    assert_eq!(root_plot(&chart).classes, ["report", "a", "b"]);
}

/// A scatter plot whose layer declares a magnitude filter over `a`, which writes `filters`.
fn filtered() -> Chart {
    let slider = RangeSliderWidget::builder()
        .target(WidgetTarget::builder().selection("filters").column("a"))
        .domain(WidgetDataDomain::builder());
    let widget = Widget::range_slider(slider).key("magnitude");
    let plot =
        Plot::cartesian(CartesianPlot::builder().layers([layer(points()).widgets([widget])]));
    Chart::builder()
        .root(plot)
        .selections([Selection::builder()
            .name("filters")
            .resolution(SelectionResolution::Intersect)])
        .infer_scales()
        .build()
        .unwrap()
}

/// Charts built apart that each declare one filter combine into a chart with one widget.
#[test]
fn combined_charts_share_the_widgets_that_they_declare() {
    for combined in [
        chart_of(Plot::builder().overlay([filtered(), filtered()])).unwrap(),
        chart_of(row().children([filtered(), filtered()])).unwrap(),
    ] {
        let resolved = combined.resolve();
        assert_eq!(resolved.widgets.len(), 1);
        assert_eq!(resolved.widgets[0].members.len(), 2);
        assert!(resolved.widgets[0].anchor.is_none());
    }
}
