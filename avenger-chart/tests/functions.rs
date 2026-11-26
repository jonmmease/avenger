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
        channel_function, channel_functions, composition, constructor, mark, Arg, ArgError,
        ChannelArg, Child, ChildOrder, Class, Expected, FormatSyntaxes, Kind, MarkTemplate, Reason,
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

/// No format syntaxes, as for charts without patterns.
const NONE: FormatSyntaxes = FormatSyntaxes {
    number: "",
    datetime: "",
};

fn chart(layer: CartesianLayerBuilder) -> Chart {
    Chart::builder(Plot::cartesian([layer])).build().unwrap()
}

#[test]
fn a_constructor_builds_its_fields_and_picks_a_member_by_the_type_of_its_value() {
    let fragment = call(
        "scale",
        vec![("zero", Arg::Bool(false)), ("nice", Arg::Int(5))],
    )
    .unwrap();
    assert_eq!(fragment.r#type, "avenger.chart.v1.Scale");
    let scale = proto::Scale::decode(fragment.message).unwrap();
    assert_eq!(scale.zero, Some(false));
    assert_eq!(scale.nice, Some(proto::scale::Nice::NiceCount(5.0)));
}

#[test]
fn a_name_picks_a_member_without_content() {
    let fragment = call("scale", vec![("type", Arg::Str("log".into()))]).unwrap();
    let scale = proto::Scale::decode(fragment.message).unwrap();
    assert!(matches!(scale.r#type, Some(proto::scale::Type::Log(_))));
}

#[test]
fn the_constructor_of_a_oneof_sets_the_oneof_of_its_parameter() {
    let log = call("log_scale", vec![("base", Arg::Int(2))]).unwrap();
    let kind = call("scale.type", vec![("log", Arg::Fragment(log))]).unwrap();
    assert_eq!(kind.r#type, "avenger.chart.v1.Scale.type");
    let fragment = call(
        "scale",
        vec![("type", Arg::Fragment(kind)), ("zero", Arg::Bool(true))],
    )
    .unwrap();
    let scale = proto::Scale::decode(fragment.message).unwrap();
    let Some(proto::scale::Type::Log(log)) = scale.r#type else {
        panic!("a log scale: {:?}", scale.r#type);
    };
    assert_eq!(log.base, Some(2.0));
    assert_eq!(scale.zero, Some(true));
    // A fragment of a whole scale is not a value of its type.
    let whole = call("scale", vec![]).unwrap();
    let error = call("scale", vec![("type", Arg::Fragment(whole))]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "`type` takes a name (linear, log, pow, symlog, time, band, point, nested_band, ordinal, \
         quantile, quantize, threshold), or `type(...)`, not a fragment of `Scale`"
    );
}

#[test]
fn the_first_parameter_takes_the_shorthands_of_its_constructor() {
    let scale = constructor("scale").unwrap();
    assert_eq!(scale.params[0].name, "value");
    assert!(scale.params[0].positional);
    let shorthand = scale.build(&[Some(Arg::Str("log".into()))]).unwrap();
    let named = call("scale", vec![("type", Arg::Str("log".into()))]).unwrap();
    assert_eq!(shorthand, named);
    // A pin's value is its first parameter, and its position is required.
    let pin = constructor("domain_pin").unwrap();
    assert_eq!(pin.params[0].name, "value");
    let error = pin.build(&[Some(Arg::Int(1000))]).unwrap_err();
    assert_eq!(error.to_string(), "`position` is required");
    // A shorthand and a named argument of its field fill one field twice.
    let error = constructor("title")
        .unwrap()
        .build(&[Some(Arg::Str("Mass".into())), Some(Arg::Str("Size".into()))])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "`text` has a value from the first argument already"
    );
}

#[test]
fn an_error_holds_its_path_and_the_inputs_of_its_parameter() {
    let error = call("scale", vec![("nice", Arg::Str("x".into()))]).unwrap_err();
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
    assert_eq!(
        classes,
        [Class::Bool, Class::Int, Class::Float, Class::Fragment]
    );
    assert_eq!(
        error.to_string(),
        "`nice` takes a Boolean, an integer, a floating-point number, or `nice(...)`, not a \
         string"
    );
}

#[test]
fn a_mark_call_builds_the_chart_of_its_builder() {
    let mut call = mark("mark_bar").expect("a mark").call();
    call.channel("x", ChannelArg::Field("c".into()), None, None, &[], &[])
        .unwrap();
    call.channel("y", ChannelArg::Field("a".into()), None, None, &[], &[])
        .unwrap();
    let template = call.bind().unwrap();
    assert_eq!(template.columns(), ["c", "a"]);
    let expected = chart(CartesianLayer::bar(
        table(),
        BarMark::builder().x("c").y("a"),
    ));
    assert_eq!(template.chart(table(), NONE).unwrap(), expected);
}

#[test]
fn bare_arguments_take_the_kinds_of_their_channels() {
    let value = |array: ArrayRef| ChannelArg::Value(array);
    let mut call = mark("mark_symbol").expect("a mark").call();
    call.channel("x", ChannelArg::Field("a".into()), None, None, &[], &[])
        .unwrap();
    call.channel(
        "y",
        value(Arc::new(Float64Array::from(vec![2.0]))),
        None,
        None,
        &[],
        &[],
    )
    .unwrap();
    call.channel(
        "fill",
        value(Arc::new(StringArray::from(vec!["red"]))),
        None,
        None,
        &[],
        &[],
    )
    .unwrap();
    call.channel(
        "color",
        ChannelArg::Field("c".into()),
        Some(Kind::Direct),
        None,
        &[],
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
    assert_eq!(template.chart(table(), NONE).unwrap(), expected);
}

/// The arguments of the options of `symbol` from named arguments.
fn mark_options(named: Vec<(&str, Arg)>) -> Vec<Option<Arg>> {
    let options = mark("mark_symbol").expect("a mark").options;
    let mut args: Vec<Option<Arg>> = vec![None; options.len()];
    for (param, arg) in named {
        let index = (options.iter())
            .position(|p| p.name == param)
            .expect("a mark option");
        args[index] = Some(arg);
    }
    args
}

/// A symbol call of `table` with the mark options `named`.
fn symbols_with(named: Vec<(&str, Arg)>) -> Chart {
    let mut call = mark("mark_symbol").expect("a mark").call();
    call.channel("x", ChannelArg::Field("a".into()), None, None, &[], &[])
        .unwrap();
    call.channel("y", ChannelArg::Field("a".into()), None, None, &[], &[])
        .unwrap();
    call.options(&mark_options(named)).unwrap();
    call.bind().unwrap().chart(table(), NONE).unwrap()
}

/// The tools `navigate()` and `tooltip()`.
fn navigate_and_tooltip() -> Arg {
    let tool = |kind_name: &str| {
        let kind = call(kind_name, vec![]).unwrap();
        Some(Arg::Fragment(
            call("tool", vec![(kind_name, Arg::Fragment(kind))]).unwrap(),
        ))
    };
    Arg::List(vec![tool("navigate"), tool("tooltip")])
}

/// The plot of the root of `chart`.
fn root_plot(chart: &Chart) -> &proto::Plot {
    let Some(proto::node::Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot")
    };
    plot
}

/// The first layer of the root plot of `chart`.
fn first_layer(chart: &Chart) -> &proto::CartesianLayer {
    let Some(proto::plot::Coord::Cartesian(cartesian)) = &root_plot(chart).coord else {
        panic!("a Cartesian plot")
    };
    &cartesian.layers[0]
}

#[test]
fn a_mark_call_places_its_options_on_its_plot_and_its_layer() {
    let chart = symbols_with(vec![
        ("title", Arg::Str("Cars".into())),
        ("tools", navigate_and_tooltip()),
        ("width", Arg::Float(200.0)),
        ("id", Arg::Str("points".into())),
        ("classes", Arg::List(vec![Some(Arg::Str("muted".into()))])),
    ]);
    let plot = root_plot(&chart);
    assert_eq!(plot.title.as_ref().unwrap().text, "Cars");
    assert_eq!(plot.width, Some(200.0));
    // The navigation stays on the plot, and the tooltip acts on the call's layer, which takes the
    // id. The plot has none, so its path identifies it.
    assert_eq!(plot.id, "");
    assert!(matches!(
        plot.tools.as_slice(),
        [proto::Tool {
            kind: Some(proto::tool::Kind::Navigate(_)),
        }]
    ));
    let layer = first_layer(&chart);
    assert_eq!(layer.id, "points");
    assert_eq!(layer.classes, ["muted"]);
    assert!(matches!(
        layer.tools.as_slice(),
        [proto::LayerTool {
            kind: Some(proto::layer_tool::Kind::Tooltip(_)),
        }]
    ));
    // Without an id, the plot and the layer have none, and their paths identify them.
    let chart = symbols_with(vec![("tools", navigate_and_tooltip())]);
    assert_eq!(root_plot(&chart).id, "");
    assert_eq!(first_layer(&chart).id, "");
    assert_eq!(first_layer(&chart).tools.len(), 1);
}

#[test]
fn a_mark_parameter_wins_over_an_option_of_its_name() {
    let names = |name: &str| -> Vec<&str> {
        let options = mark(name).expect("a mark").options;
        options.iter().map(|option| option.name).collect()
    };
    assert_eq!(
        names("mark_symbol"),
        [
            "title",
            "attribution",
            "tools",
            "width",
            "height",
            "id",
            "classes"
        ]
    );
    // The `width` channel of `text` wins over the width of its plot.
    assert_eq!(
        names("mark_text"),
        ["title", "attribution", "tools", "height", "id", "classes"]
    );
    assert_eq!(
        names("mark_image"),
        ["title", "attribution", "tools", "id", "classes"]
    );
}

#[test]
fn a_layer_composition_merges_the_options_of_its_charts() {
    let layer = composition("layer").unwrap();
    let titled = symbols_with(vec![
        ("title", Arg::Str("Cars".into())),
        ("tools", navigate_and_tooltip()),
    ]);
    let plain = symbols_with(vec![("tools", navigate_and_tooltip())]);
    let child = |chart: &Chart| Child {
        chart: Some(chart.clone()),
        key: None,
    };
    let chart = layer
        .compose(
            None,
            vec![child(&titled), child(&plain)],
            ChildOrder::AsGiven,
            &[],
            NONE,
        )
        .unwrap();
    let plot = root_plot(&chart);
    // One title, the two equal navigations as one, and the tooltip of each layer.
    assert_eq!(plot.title.as_ref().unwrap().text, "Cars");
    assert_eq!(plot.tools.len(), 1);
    let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("a Cartesian plot")
    };
    assert!(cartesian.layers.iter().all(|layer| layer.tools.len() == 1));
    // The ids of the charts name their layers, and the composition's id names the plot.
    let named = |id: &str| {
        symbols_with(vec![
            ("id", Arg::Str(id.into())),
            ("tools", navigate_and_tooltip()),
        ])
    };
    let options = layer.params.iter().position(|p| p.name == "id").unwrap();
    let mut args = vec![None; layer.params.len()];
    args[options] = Some(Arg::Str("both".into()));
    let charts = vec![child(&named("a")), child(&named("b"))];
    let chart = layer
        .compose(None, charts, ChildOrder::AsGiven, &args, NONE)
        .unwrap();
    assert_eq!(root_plot(&chart).id, "both");
    let Some(proto::plot::Coord::Cartesian(cartesian)) = &root_plot(&chart).coord else {
        panic!("a Cartesian plot")
    };
    let ids: Vec<&str> = (cartesian.layers.iter())
        .map(|layer| layer.id.as_str())
        .collect();
    assert_eq!(ids, ["a", "b"]);
}

#[test]
fn the_rules_of_a_mark_fail_its_call() {
    let mut call = mark("mark_geo_symbol").expect("a mark").call();
    call.channel(
        "longitude",
        ChannelArg::Field("a".into()),
        None,
        None,
        &[],
        &[],
    )
    .unwrap();
    call.channel(
        "latitude",
        ChannelArg::Field("a".into()),
        None,
        None,
        &[],
        &[],
    )
    .unwrap();
    let error = call.bind().unwrap_err();
    assert!(matches!(error.reason, Reason::Mark(_)), "{error:?}");
    assert!(
        error.to_string().contains("geo_symbol() needs `coord`"),
        "{error}"
    );

    let mut call = mark("mark_text").expect("a mark").call();
    let error = (call.channel(
        "text",
        ChannelArg::Field("c".into()),
        Some(Kind::Scaled),
        None,
        &[],
        &[],
    ))
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("datum() needs a channel with a scale"),
        "{error}"
    );
}

#[test]
fn the_options_of_the_composition_nodes_have_constructors() {
    // A number is a track's length, and a track takes a share and a maximum by name.
    let length = call("track", vec![("length", Arg::Int(120))]).unwrap();
    let length = proto::Track::decode(length.message).unwrap();
    assert_eq!(length.size, Some(proto::track::Size::Length(120.0)));
    let share = call(
        "track",
        vec![("share", Arg::Int(2)), ("max", Arg::Int(600))],
    )
    .unwrap();
    let share = proto::Track::decode(share.message).unwrap();
    assert_eq!(share.size, Some(proto::track::Size::Share(2.0)));
    assert_eq!(share.max, Some(600.0));

    let parts = call("part_rules", vec![("legend", Arg::Str("keep".into()))]).unwrap();
    let parts = proto::PartRules::decode(parts.message).unwrap();
    assert_eq!(parts.legend, proto::PartRuleKind::Keep as i32);

    // A facet constructor takes no column: the composition that builds a facet sets it from its key.
    let facet = call("facet", vec![("orient", Arg::Str("bottom".into()))]).unwrap();
    let facet = proto::Facet::decode(facet.message).unwrap();
    assert!(facet.column.is_empty());
    assert_eq!(facet.orient, proto::Side::Bottom as i32);
    let params = &constructor("facet").unwrap().params;
    assert!(params.iter().all(|param| param.name != "column"));
}

#[test]
fn a_track_takes_a_length_or_a_share() {
    let error = call(
        "track",
        vec![("length", Arg::Int(200)), ("share", Arg::Int(2))],
    )
    .unwrap_err();
    assert!(
        matches!(error.reason, Reason::Exclusive { .. }),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "`share` sets the same choice as `length`, so a call gives one of them"
    );
}

#[test]
fn a_resolve_names_its_channels_and_its_independent_widgets() {
    let resolve = call(
        "resolve",
        vec![
            ("y", Arg::Str("independent".into())),
            ("projection", Arg::Str("shared".into())),
            (
                "independent_widgets",
                Arg::List(vec![Some(Arg::Str("w".into()))]),
            ),
        ],
    )
    .unwrap();
    let resolve = proto::Resolve::decode(resolve.message).unwrap();
    assert_eq!(resolve.y, proto::Sharing::Independent as i32);
    assert_eq!(resolve.x, proto::Sharing::Unspecified as i32);
    assert_eq!(resolve.projection, proto::Sharing::Shared as i32);
    assert_eq!(resolve.independent_widgets, ["w"]);
    let params = &constructor("resolve").unwrap().params;
    assert!(params.iter().all(|param| param.name != "x2"));
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
        .compose(None, charts, ChildOrder::AsGiven, &[], NONE)
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
        .compose(None, charts, ChildOrder::AsGiven, &[], NONE)
        .unwrap_err();
    assert!(matches!(error.reason, Reason::Composition(_)), "{error:?}");
    assert!(
        error.to_string().contains("has a concat at its root"),
        "{error}"
    );
}

#[test]
fn a_concat_takes_the_options_of_its_node() {
    for (name, direction) in [("hconcat", Direction::Row), ("vconcat", Direction::Column)] {
        let concat = composition(name).unwrap();
        let mut options = vec![None; concat.params.len()];
        let at = |param: &str| concat.params.iter().position(|p| p.name == param).unwrap();
        options[at("id")] = Some(Arg::Str("pair".into()));
        options[at("gap")] = Some(Arg::Int(8));
        options[at("tracks")] = Some(Arg::List(vec![
            Some(Arg::Int(200)),
            Some(Arg::Float(400.5)),
        ]));
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
            .compose(None, charts, ChildOrder::AsGiven, &options, NONE)
            .unwrap();
        let Some(proto::node::Node::Concat(root)) = &chart.root().node else {
            panic!("a concat");
        };
        assert_eq!(root.direction, direction as i32);
        assert_eq!(root.id, "pair");
        assert_eq!(root.gap, Some(8.0));
        let lengths: Vec<_> = root.tracks.iter().map(|track| track.size).collect();
        assert_eq!(
            lengths,
            [
                Some(proto::track::Size::Length(200.0)),
                Some(proto::track::Size::Length(400.5))
            ]
        );
        assert_eq!(root.children.len(), 2);
    }
}

#[test]
fn a_facet_orders_its_keys_with_null_last_and_draws_an_empty_cell_for_a_null_chart() {
    let facet = composition("facet_column").unwrap();
    let charts = vec![
        child(Some(plot("p")), Some("Torgersen")),
        child(Some(plot("p")), None),
        child(None, Some("Biscoe")),
        child(Some(plot("p")), Some("Dream")),
    ];
    let chart = facet
        .compose(Some("island"), charts, ChildOrder::ByKey, &[], NONE)
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
        .compose(Some("k"), charts, ChildOrder::ByKey, &[], NONE)
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
        .compose(Some("k"), charts, ChildOrder::ByKey, &options, NONE)
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
    let cols = composition("facet_column").unwrap();
    let row = |values: &[&str]| {
        let charts = values
            .iter()
            .map(|v| child(Some(plot("p")), Some(v)))
            .collect();
        cols.compose(Some("island"), charts, ChildOrder::ByKey, &[], NONE)
            .unwrap()
    };
    // The male row lacks Dream.
    let charts = vec![
        child(Some(row(&["Biscoe", "Dream"])), Some("female")),
        child(Some(row(&["Biscoe"])), Some("male")),
    ];
    let grid = composition("facet_row")
        .unwrap()
        .compose(Some("sex"), charts, ChildOrder::ByKey, &[], NONE)
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
        .compose(Some("k"), charts, ChildOrder::AsGiven, &[], NONE)
        .unwrap();
    let Some(proto::node::Node::Wrap(root)) = &chart.root().node else {
        panic!("a wrap");
    };
    assert_eq!(root.facet.as_ref().unwrap().column, "k");
    let order: Vec<Option<String>> = (cells(&chart, chart.root(), "k").into_iter())
        .map(|(value, _)| value)
        .collect();
    assert_eq!(order, [Some("b".to_string()), Some("a".to_string())]);

    for name in ["layer", "hconcat", "facet_column"] {
        let chart = composition(name)
            .unwrap()
            .compose(Some("k"), Vec::new(), ChildOrder::ByKey, &[], NONE)
            .unwrap();
        assert!(matches!(
            chart.root().node,
            Some(proto::node::Node::Empty(_))
        ));
    }
}

#[test]
fn a_binding_names_the_format_syntaxes_of_each_chart_that_it_builds() {
    let googlesql = FormatSyntaxes {
        number: "googlesql",
        datetime: "googlesql",
    };
    let syntaxes = |chart: &Chart| {
        let message = chart.message();
        (
            message.number_format_syntax.clone(),
            message.datetime_format_syntax.clone(),
        )
    };
    let named = ("googlesql".to_string(), "googlesql".to_string());
    let mut call = mark("mark_symbol").expect("a mark").call();
    call.channel("x", ChannelArg::Field("a".into()), None, None, &[], &[])
        .unwrap();
    call.channel("y", ChannelArg::Field("a".into()), None, None, &[], &[])
        .unwrap();
    let symbols = call.bind().unwrap().chart(table(), googlesql).unwrap();
    assert_eq!(syntaxes(&symbols), named);

    // A composition names them too, with children or without, and a child's syntaxes agree.
    let layered = composition("layer")
        .unwrap()
        .compose(
            None,
            vec![
                child(Some(symbols.clone()), None),
                child(Some(symbols.clone()), None),
            ],
            ChildOrder::AsGiven,
            &[],
            googlesql,
        )
        .unwrap();
    assert_eq!(syntaxes(&layered), named);
    let facet = composition("facet_column")
        .unwrap()
        .compose(
            Some("k"),
            vec![child(None, Some("a"))],
            ChildOrder::ByKey,
            &[],
            googlesql,
        )
        .unwrap();
    assert_eq!(syntaxes(&facet), named);
    let empty = composition("hconcat")
        .unwrap()
        .compose(None, Vec::new(), ChildOrder::AsGiven, &[], googlesql)
        .unwrap();
    assert_eq!(syntaxes(&empty), named);

    // A child of other syntaxes is a conflict.
    let d3 = FormatSyntaxes {
        number: "d3",
        datetime: "d3",
    };
    let error = composition("vconcat")
        .unwrap()
        .compose(
            None,
            vec![child(Some(symbols), None)],
            ChildOrder::AsGiven,
            &[],
            d3,
        )
        .unwrap_err();
    assert!(error.to_string().contains("conflict"), "{error}");
}

#[test]
fn each_channel_with_a_scale_has_a_function_with_the_options_of_its_channel() {
    let names: Vec<&str> = channel_functions().iter().map(|f| f.name).collect();
    assert_eq!(
        names,
        [
            "x",
            "y",
            "x2",
            "y2",
            "theta",
            "radius",
            "theta2",
            "radius2",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "stroke_dasharray",
            "size",
            "shape",
            "angle",
            "pattern"
        ]
    );
    let options = |name: &str| -> Vec<&str> {
        let function = channel_function(name).expect("a channel function");
        function.options.iter().map(|option| option.name).collect()
    };
    assert_eq!(
        options("x"),
        [
            "scale",
            "axis",
            "title",
            "band_position",
            "band_level",
            "selection_field"
        ]
    );
    assert!(options("radius2").is_empty());
    assert_eq!(
        options("size"),
        ["scale", "legend", "title", "selection_field"]
    );
    // No channel function has the name of a mark.
    for function in channel_functions() {
        assert!(mark(function.name).is_none(), "{}", function.name);
    }
}

#[test]
fn a_channel_function_sets_its_options_and_only_its_own_channel() {
    let mut call = mark("mark_symbol").expect("a mark").call();
    call.channel("x", ChannelArg::Field("a".into()), None, None, &[], &[])
        .unwrap();
    call.channel("y", ChannelArg::Field("a".into()), None, None, &[], &[])
        .unwrap();
    // The options in the order of the function's, `scale`, `legend`, then `title`.
    call.channel(
        "color",
        ChannelArg::Field("c".into()),
        None,
        channel_function("color"),
        &[None, None, Some(Arg::Str("C".into()))],
        &[],
    )
    .unwrap();
    let error = call
        .channel(
            "fill",
            ChannelArg::Field("c".into()),
            None,
            channel_function("color"),
            &[],
            &[],
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("`fill` of mark_symbol() takes `fill(...)`, not `color(...)`"),
        "{error}"
    );
    let chart = call.bind().unwrap().chart(table(), NONE).unwrap();
    let plot = root_plot(&chart);
    let layer = avenger_chart::LayerRef::of(plot).remove(0);
    let encoding = (layer.mark().unwrap())
        .encoding(proto::Channel::Color)
        .unwrap();
    assert_eq!(encoding.title.as_deref(), Some("C"), "{encoding:?}");
    // `datum` takes a constant, since a column passes through the scale without it.
    let mut call = mark("mark_symbol").expect("a mark").call();
    let error = (call.channel(
        "color",
        ChannelArg::Field("c".into()),
        Some(Kind::Scaled),
        None,
        &[],
        &[],
    ))
    .unwrap_err();
    assert!(
        error.to_string().contains("datum() takes a constant"),
        "{error}"
    );
}

#[test]
fn a_mark_template_round_trips_through_its_bytes() {
    let mut call = mark("mark_bar").expect("a mark").call();
    call.channel("x", ChannelArg::Field("c".into()), None, None, &[], &[])
        .unwrap();
    call.channel("y", ChannelArg::Field("a".into()), None, None, &[], &[])
        .unwrap();
    let template = call.bind().unwrap();
    let decoded = MarkTemplate::from_bytes(&template.to_bytes()).unwrap();
    assert_eq!(decoded, template);
    assert_eq!(
        decoded.chart(table(), NONE).unwrap(),
        template.chart(table(), NONE).unwrap()
    );
    assert!(MarkTemplate::from_bytes(b"not a template").is_err());
}
