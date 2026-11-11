//! The functions of the `functions` feature, called from Rust without SQL, as a binding of another
//! interface calls them: constructors build the fragments of their builders, and mark calls build
//! the charts of their builders.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
};
use avenger_chart::{
    functions::{constructor, mark, Arg, ArgError, ChannelArg, Class, Expected, Kind, Reason},
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

    // A facet constructor takes no column: a facet aggregate sets it from its key.
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
