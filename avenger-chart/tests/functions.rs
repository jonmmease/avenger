//! The functions of the `functions` feature, called from Rust without SQL, as a binding of another
//! interface calls them: constructors build the fragments of their builders, and mark calls build
//! the charts of their builders.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
};
use avenger_chart::{
    functions::{
        composition, constructor, mark, Arg, ArgError, ChannelArg, Child, ChildOrder, Class,
        Expected, Kind, Reason,
    },
    prelude::*,
    proto::{self, Fragment},
};
use prost::Message;

/// The arguments of the constructor `name` from named arguments.
fn call(name: &str, named: Vec<(&str, Arg)>) -> Result<Fragment, ArgError> {
    let constructor = constructor(name).expect("a constructor");
    let mut args: Vec<Option<Arg>> = vec![None; constructor.params.len()];
    for (param, arg) in named {
        let index = (constructor.params.iter())
            .position(|p| p.name == param)
            .expect("a parameter");
        args[index] = Some(arg);
    }
    constructor.build(&args)
}

fn table() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("a", DataType::Float64, false),
        Field::new("c", DataType::Utf8, false),
    ]));
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Float64Array::from(vec![1.0, 2.0])),
        Arc::new(StringArray::from(vec!["p", "q"])),
    ];
    RecordBatch::try_new(schema, columns).unwrap()
}

fn chart(layer: CartesianLayerBuilder) -> Chart {
    Chart::builder(Plot::cartesian([layer])).build().unwrap()
}

#[test]
fn a_constructor_builds_its_fields_and_picks_a_member_by_the_type_of_its_value() {
    let fragment = call(
        "interval_fit",
        vec![("zero", Arg::Bool(false)), ("nice", Arg::Int(5))],
    )
    .unwrap();
    assert_eq!(fragment.r#type, "avenger.chart.v1.IntervalFit");
    let fit = proto::IntervalFit::decode(fragment.message).unwrap();
    assert_eq!(fit.zero, Some(false));
    assert_eq!(fit.nice, Some(proto::interval_fit::Nice::TickCount(5.0)));
}

#[test]
fn a_name_picks_a_member_without_content() {
    let fragment = call("scale", vec![("type", Arg::Str("log".into()))]).unwrap();
    let scale = proto::Scale::decode(fragment.message).unwrap();
    assert!(matches!(scale.r#type, Some(proto::scale::Type::Log(_))));
}

#[test]
fn a_fragment_reaches_its_field_through_a_chain() {
    let fit = call("interval_fit", vec![("zero", Arg::Bool(false))]).unwrap();
    let fragment = call("scale", vec![("domain", Arg::Fragment(fit))]).unwrap();
    let scale = proto::Scale::decode(fragment.message).unwrap();
    let domain = scale.domain.and_then(|domain| domain.domain);
    let Some(proto::domain::Domain::Data(data)) = domain else {
        panic!("a data domain: {domain:?}");
    };
    let fit = data.fit.and_then(|fit| fit.fit);
    let Some(proto::domain_fit::Fit::Interval(fit)) = fit else {
        panic!("an interval fit: {fit:?}");
    };
    assert_eq!(fit.zero, Some(false));
}

#[test]
fn an_error_holds_its_path_and_the_inputs_of_its_parameter() {
    let error = call("interval_fit", vec![("nice", Arg::Str("x".into()))]).unwrap_err();
    assert_eq!(error.path, ["nice"]);
    let Reason::Rejected {
        expected: Expected::Inputs(accepts),
        found: Class::Str,
        ..
    } = &error.reason
    else {
        panic!("a rejected string: {error:?}");
    };
    let classes: Vec<Class> = accepts.iter().map(|accept| accept.class).collect();
    assert_eq!(classes, [Class::Bool, Class::Int, Class::Float]);
    assert_eq!(
        error.to_string(),
        "`nice` takes a Boolean, an integer, or a floating-point number, not a string"
    );
}

#[test]
fn a_mark_call_builds_the_chart_of_its_builder() {
    let mut call = mark("bar").expect("a mark").call();
    call.channel("x", ChannelArg::Field("c".into()), None, &[])
        .unwrap();
    call.channel("y", ChannelArg::Field("a".into()), None, &[])
        .unwrap();
    let template = call.bind().unwrap();
    assert_eq!(template.columns(), ["c", "a"]);
    let expected = chart(CartesianLayer::bar(
        table(),
        BarMark::builder().x("c").y("a"),
    ));
    assert_eq!(template.chart(table()).unwrap(), expected);
}

#[test]
fn bare_arguments_take_the_kinds_of_their_channels() {
    let value = |array: ArrayRef| ChannelArg::Value(array);
    let mut call = mark("symbol").expect("a mark").call();
    call.channel("x", ChannelArg::Field("a".into()), None, &[])
        .unwrap();
    call.channel(
        "y",
        value(Arc::new(Float64Array::from(vec![2.0]))),
        None,
        &[],
    )
    .unwrap();
    call.channel(
        "fill",
        value(Arc::new(StringArray::from(vec!["red"]))),
        None,
        &[],
    )
    .unwrap();
    call.channel(
        "color",
        ChannelArg::Field("c".into()),
        Some(Kind::Direct),
        &[],
    )
    .unwrap();
    let template = call.bind().unwrap();
    assert_eq!(template.columns(), ["a", "c"]);
    let symbol = SymbolMark::builder()
        .x(Encoding::scaled_field("a"))
        .y(Encoding::scaled_value(Value::scalar(Arc::new(
            Float64Array::from(vec![2.0]),
        ))))
        .fill(Encoding::direct_value(Value::scalar(Arc::new(
            StringArray::from(vec!["red"]),
        ))))
        .color(Encoding::direct_field("c"));
    let expected = chart(CartesianLayer::symbol(table(), symbol));
    assert_eq!(template.chart(table()).unwrap(), expected);
}

#[test]
fn the_rules_of_a_mark_fail_its_call() {
    let mut call = mark("geo_symbol").expect("a mark").call();
    call.channel("longitude", ChannelArg::Field("a".into()), None, &[])
        .unwrap();
    call.channel("latitude", ChannelArg::Field("a".into()), None, &[])
        .unwrap();
    let error = call.bind().unwrap_err();
    assert!(matches!(error.reason, Reason::Mark(_)), "{error:?}");
    assert!(
        error.to_string().contains("geo_symbol() needs `coord`"),
        "{error}"
    );

    let mut call = mark("text").expect("a mark").call();
    let error = (call.channel(
        "text",
        ChannelArg::Field("c".into()),
        Some(Kind::Scaled),
        &[],
    ))
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("scaled() needs a channel with a scale"),
        "{error}"
    );
}

#[test]
fn the_options_of_the_composition_nodes_have_constructors() {
    let track = call("track", vec![("size", Arg::Int(120))]).unwrap();
    let layout = call(
        "concat_layout",
        vec![
            ("gap", Arg::Int(8)),
            ("tracks", Arg::List(vec![Some(Arg::Fragment(track))])),
        ],
    )
    .unwrap();
    let layout = proto::ConcatLayout::decode(layout.message).unwrap();
    assert_eq!(layout.gap, Some(8.0));
    assert_eq!(layout.tracks.len(), 1);

    // A facet constructor takes no column: the composition that builds a facet sets it from its key.
    let facet = call("facet", vec![("side", Arg::Str("bottom".into()))]).unwrap();
    let facet = proto::Facet::decode(facet.message).unwrap();
    assert!(facet.column.is_empty());
    assert_eq!(facet.side, proto::Side::Bottom as i32);
    let params = &constructor("facet").unwrap().params;
    assert!(params.iter().all(|param| param.name != "column"));
}

#[test]
fn a_string_key_of_a_resolve_rule_names_a_channel() {
    let sharing = ("sharing", Arg::Str("independent".into()));
    let channel = call(
        "resolve",
        vec![("key", Arg::Str("y".into())), sharing.clone()],
    )
    .unwrap();
    let channel = proto::Resolve::decode(channel.message).unwrap();
    assert_eq!(
        channel.key,
        Some(proto::resolve::Key::Channel(proto::Channel::Y as i32))
    );
    let widget = call("widget_key", vec![("key", Arg::Str("w".into()))]).unwrap();
    let widget = call("resolve", vec![("key", Arg::Fragment(widget)), sharing]).unwrap();
    let widget = proto::Resolve::decode(widget.message).unwrap();
    assert_eq!(
        widget.key,
        Some(proto::resolve::Key::Widget(proto::WidgetKey {
            key: "w".into()
        }))
    );
}

/// A chart of one plot of symbols, with the id `id`.
fn plot(id: &str) -> Chart {
    let layer = CartesianLayer::symbol(table(), SymbolMark::builder().x("c").y("a"));
    Chart::builder(Plot::cartesian([layer]).id(id))
        .build()
        .unwrap()
}

/// The children of a chart in its order, with each child's member.
fn children(node: &proto::Node) -> &[proto::Node] {
    match &node.node {
        Some(proto::node::Node::Concat(concat)) => &concat.children,
        Some(proto::node::Node::Wrap(wrap)) => &wrap.children,
        other => panic!("a concat or a wrap, not {other:?}"),
    }
}

/// The values of the partition column `column` of the children of `chart`'s root, as strings,
/// with `None` for a null value, and whether each child is empty.
fn cells(chart: &Chart, node: &proto::Node, column: &str) -> Vec<(Option<String>, bool)> {
    children(node)
        .iter()
        .map(|child| {
            let (partition, empty) = match &child.node {
                Some(proto::node::Node::Plot(plot)) => (&plot.partition, false),
                Some(proto::node::Node::Concat(concat)) => (&concat.partition, false),
                Some(proto::node::Node::Wrap(wrap)) => (&wrap.partition, false),
                Some(proto::node::Node::Empty(empty)) => (&empty.partition, true),
                None => panic!("a node"),
            };
            let value = partition
                .iter()
                .find(|value| value.column == column)
                .unwrap();
            let value = chart.partition_value(value);
            let text = (!value.is_null(0))
                .then(|| arrow::util::display::array_value_to_string(value.as_ref(), 0).unwrap());
            (text, empty)
        })
        .collect()
}

fn key(value: Option<&str>) -> Option<ArrayRef> {
    Some(Arc::new(StringArray::from(vec![value])))
}

fn child(chart: Option<Chart>, value: Option<&str>) -> Child {
    Child {
        chart,
        key: key(value),
    }
}

#[test]
fn layer_overlays_the_plots_of_its_charts() {
    let layer = composition("layer").unwrap();
    let charts = vec![
        Child {
            chart: Some(plot("a")),
            key: None,
        },
        Child {
            chart: None,
            key: None,
        },
        Child {
            chart: Some(plot("a")),
            key: None,
        },
    ];
    let chart = layer
        .compose(None, charts, ChildOrder::AsGiven, &[])
        .unwrap();
    let expected = Chart::builder(Plot::overlay([plot("a"), plot("a")]))
        .build()
        .unwrap();
    assert_eq!(chart, expected);

    let concat = Chart::builder(Concat::new(Direction::Row).children([plot("a")]))
        .build()
        .unwrap();
    let charts = vec![Child {
        chart: Some(concat),
        key: None,
    }];
    let error = layer
        .compose(None, charts, ChildOrder::AsGiven, &[])
        .unwrap_err();
    assert!(matches!(error.reason, Reason::Composition(_)), "{error:?}");
    assert!(
        error.to_string().contains("has a concat at its root"),
        "{error}"
    );
}

#[test]
fn a_concat_takes_the_options_of_its_node() {
    let gap = call("concat_layout", vec![("gap", Arg::Int(8))]).unwrap();
    for (name, direction) in [("hconcat", Direction::Row), ("vconcat", Direction::Column)] {
        let concat = composition(name).unwrap();
        let mut options = vec![None; concat.params.len()];
        let at = |param: &str| concat.params.iter().position(|p| p.name == param).unwrap();
        options[at("id")] = Some(Arg::Str("pair".into()));
        options[at("layout")] = Some(Arg::Fragment(gap.clone()));
        let charts = vec![
            Child {
                chart: Some(plot("a")),
                key: None,
            },
            Child {
                chart: Some(plot("b")),
                key: None,
            },
        ];
        let chart = concat
            .compose(None, charts, ChildOrder::AsGiven, &options)
            .unwrap();
        let Some(proto::node::Node::Concat(root)) = &chart.root().node else {
            panic!("a concat");
        };
        assert_eq!(root.direction, direction as i32);
        assert_eq!(root.id, "pair");
        assert_eq!(
            root.layout.as_ref().and_then(|layout| layout.gap),
            Some(8.0)
        );
        assert_eq!(root.children.len(), 2);
    }
}

#[test]
fn a_facet_orders_its_keys_with_null_last_and_draws_an_empty_cell_for_a_null_chart() {
    let facet = composition("facet_col").unwrap();
    let charts = vec![
        child(Some(plot("p")), Some("Torgersen")),
        child(Some(plot("p")), None),
        child(None, Some("Biscoe")),
        child(Some(plot("p")), Some("Dream")),
    ];
    let chart = facet
        .compose(Some("island"), charts, ChildOrder::ByKey, &[])
        .unwrap();
    let Some(proto::node::Node::Concat(root)) = &chart.root().node else {
        panic!("a concat");
    };
    assert_eq!(root.direction, Direction::Row as i32);
    assert_eq!(root.facet.as_ref().unwrap().column, "island");
    let expected = [
        (Some("Biscoe".to_string()), true),
        (Some("Dream".to_string()), false),
        (Some("Torgersen".to_string()), false),
        (None, false),
    ];
    assert_eq!(cells(&chart, chart.root(), "island"), expected);
}

#[test]
fn a_facet_takes_one_chart_for_each_key_value() {
    let facet = composition("facet_row").unwrap();
    let charts = vec![
        child(Some(plot("p")), Some("a")),
        child(Some(plot("p")), Some("a")),
    ];
    let error = facet
        .compose(Some("k"), charts, ChildOrder::ByKey, &[])
        .unwrap_err();
    assert!(
        error.to_string().contains("two rows have the key value a"),
        "{error}"
    );

    // The constructor takes no column, but a fragment of a facet can name one.
    let other = proto::FacetBuilder::from("other").to_fragment().unwrap();
    let mut options = vec![None; facet.params.len()];
    let at = facet.params.iter().position(|p| p.name == "facet").unwrap();
    options[at] = Some(Arg::Fragment(other));
    let charts = vec![child(Some(plot("p")), Some("a"))];
    let error = facet
        .compose(Some("k"), charts, ChildOrder::ByKey, &options)
        .unwrap_err();
    assert_eq!(error.path, ["facet"]);
    assert!(
        error.to_string().contains("names the column `other`"),
        "{error}"
    );
    // A binding checks the options before it composes rows.
    let checked = facet.check(Some("k"), &options).unwrap_err();
    assert_eq!(checked.to_string(), error.to_string());
    facet.check(Some("other"), &options).unwrap();
}

#[test]
fn a_facet_of_facets_aligns_their_cells() {
    let cols = composition("facet_col").unwrap();
    let row = |values: &[&str]| {
        let charts = values
            .iter()
            .map(|v| child(Some(plot("p")), Some(v)))
            .collect();
        cols.compose(Some("island"), charts, ChildOrder::ByKey, &[])
            .unwrap()
    };
    // The male row lacks Dream.
    let charts = vec![
        child(Some(row(&["Biscoe", "Dream"])), Some("female")),
        child(Some(row(&["Biscoe"])), Some("male")),
    ];
    let grid = composition("facet_row")
        .unwrap()
        .compose(Some("sex"), charts, ChildOrder::ByKey, &[])
        .unwrap();
    let rows = children(grid.root());
    assert_eq!(rows.len(), 2);
    let biscoe = (Some("Biscoe".to_string()), false);
    assert_eq!(
        cells(&grid, &rows[0], "island"),
        [biscoe.clone(), (Some("Dream".to_string()), false)]
    );
    assert_eq!(
        cells(&grid, &rows[1], "island"),
        [biscoe, (Some("Dream".to_string()), true)]
    );
}

#[test]
fn a_wrapped_facet_and_an_empty_composition() {
    let wrap = composition("facet_wrap").unwrap();
    let charts = vec![
        child(Some(plot("p")), Some("b")),
        child(Some(plot("p")), Some("a")),
    ];
    let chart = wrap
        .compose(Some("k"), charts, ChildOrder::AsGiven, &[])
        .unwrap();
    let Some(proto::node::Node::Wrap(root)) = &chart.root().node else {
        panic!("a wrap");
    };
    assert_eq!(root.facet.as_ref().unwrap().column, "k");
    let order: Vec<Option<String>> = (cells(&chart, chart.root(), "k").into_iter())
        .map(|(value, _)| value)
        .collect();
    assert_eq!(order, [Some("b".to_string()), Some("a".to_string())]);

    for name in ["layer", "hconcat", "facet_col"] {
        let chart = composition(name)
            .unwrap()
            .compose(Some("k"), Vec::new(), ChildOrder::ByKey, &[])
            .unwrap();
        assert!(matches!(
            chart.root().node,
            Some(proto::node::Node::Empty(_))
        ));
    }
}
