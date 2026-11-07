//! The generated builders of the messages: each rule of their generation on a real message, and the
//! building of a chart.

mod support;

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, AsArray, Date32Array, Float64Array},
    datatypes::{DataType, Float64Type},
};
use avenger_chart::{
    builder::Value,
    proto::{
        self, cartesian_layer, conditional_value, domain, domain_fit, encoding, guide, node, plot,
        predicate, scale_spec, BarMark, CartesianLayer, CartesianPlot, Chart, Concat, CustomScale,
        DataDomain, Domain, Encoding, Guide, IntervalFit, LinearScale, Orient, Plot, Predicate,
        ScaleSpec, Selected, SymbolMark,
    },
    ChartError, FORMAT_VERSION,
};
use support::points;

/// Fails unless `result` is invalid at `path`, for a reason that contains `reason`.
#[track_caller]
fn invalid<T: std::fmt::Debug>(result: Result<T, ChartError>, path: &str, reason: &str) {
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

/// A chart of one layer of symbols of `a` against `b` with `mark`'s other channels.
fn scatter(mark: proto::SymbolMarkBuilder) -> proto::ChartBuilder {
    let layer = CartesianLayer::symbol(
        mark.x(Encoding::scaled_column("a"))
            .y(Encoding::scaled_column("b")),
    )
    .table(points());
    Chart::builder().root(Plot::cartesian(CartesianPlot::builder().layers([layer])))
}

/// The symbols of the first layer of `chart`.
fn symbols(chart: &avenger_chart::Chart) -> &proto::SymbolMark {
    let Some(node::Node::Plot(plot)) = &chart.root().node else {
        panic!("the root is a plot")
    };
    let Some(plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("the plot is Cartesian")
    };
    let Some(cartesian_layer::Mark::Symbol(mark)) = &cartesian.layers[0].mark else {
        panic!("the layer draws symbols")
    };
    mark
}

#[test]
fn scalar_optional_and_enum_fields_take_their_rust_types() {
    let bar = BarMark::builder()
        .orient(Orient::Horizontal)
        .corner_radius(4.0)
        .build()
        .unwrap();
    assert_eq!(bar.orient, Orient::Horizontal as i32);
    assert_eq!(bar.corner_radius, Some(4.0));
    // An optional field that the caller does not set stays absent.
    assert_eq!(BarMark::builder().build().unwrap().corner_radius, None);
    // `false` is an explicit setting of an optional field.
    let fit = IntervalFit::builder().zero(false).build().unwrap();
    assert_eq!(fit.zero, Some(false));
}

#[test]
fn a_repeated_field_appends_in_order() {
    let concat = Concat::builder()
        .classes(["a", "b"])
        .classes(["c"])
        .build()
        .unwrap();
    assert_eq!(concat.classes, ["a", "b", "c"]);
}

#[test]
fn a_oneof_member_replaces_the_member_that_is_set() {
    let encoding = Encoding::scaled_column("a").column("b").build().unwrap();
    assert_eq!(
        encoding.value,
        Some(encoding::Value::Column("b".to_string()))
    );
    // A constructor is the member's setter on an empty builder.
    assert_eq!(
        Encoding::scaled_column("a").build().unwrap(),
        Encoding::builder().scaled_column("a").build().unwrap()
    );
}

#[test]
fn a_wrapper_converts_from_its_members() {
    let spec = ScaleSpec::builder()
        .domain(DataDomain::builder().fit(IntervalFit::builder().zero(true)))
        .build()
        .unwrap();
    let Some(Domain {
        domain: Some(domain::Domain::Data(data)),
    }) = spec.domain
    else {
        panic!("the domain is a data domain")
    };
    let Some(proto::DomainFit {
        fit: Some(domain_fit::Fit::Interval(fit)),
    }) = data.fit
    else {
        panic!("the fit is an interval fit")
    };
    assert_eq!(fit.zero, Some(true));
    // A message converts as its builder does.
    let guide = proto::GuideBuilder::from(proto::Axis::default())
        .build()
        .unwrap();
    assert!(matches!(guide.guide, Some(guide::Guide::Axis(_))));
}

#[test]
fn a_member_without_fields_takes_no_argument() {
    let spec = ScaleSpec::builder().quantile().build().unwrap();
    assert!(matches!(spec.r#type, Some(scale_spec::Type::Quantile(_))));
    assert!(matches!(
        Guide::none().build().unwrap().guide,
        Some(guide::Guide::None(_))
    ));
}

#[test]
fn a_member_of_one_repeated_field_takes_its_items() {
    let selected = || Predicate::selected(Selected::builder().selection("brush"));
    let all = Predicate::all([selected(), selected()]).build().unwrap();
    let Some(predicate::Predicate::All(list)) = all.predicate else {
        panic!("the predicate is a list of all")
    };
    assert_eq!(list.predicates.len(), 2);
}

#[test]
fn a_keyword_field_takes_a_trailing_underscore() {
    let custom = CustomScale::builder().type_("warp").build().unwrap();
    assert_eq!(custom.r#type, "warp");
}

#[test]
fn a_message_converts_to_its_builder() {
    let message = BarMark {
        corner_radius: Some(2.0),
        ..Default::default()
    };
    let bar = proto::BarMarkBuilder::from(message.clone())
        .build()
        .unwrap();
    assert_eq!(bar, message);
}

#[test]
fn values_take_the_arrow_type_of_their_rust_type() {
    let chart = scatter(
        SymbolMark::builder()
            .color(Encoding::constant("#e45756"))
            .size(Encoding::constant(5)),
    )
    .infer_scales()
    .build()
    .unwrap();
    let mark = symbols(&chart);
    let value = |encoding: &Option<Encoding>| match &encoding.as_ref().unwrap().value {
        Some(encoding::Value::Constant(value)) => chart.value(value).clone(),
        other => panic!("expected a constant, got {other:?}"),
    };
    assert_eq!(value(&mark.color).data_type(), &DataType::Utf8);
    // A literal `5` is an `i32`, without a conversion.
    assert_eq!(value(&mark.size).data_type(), &DataType::Int32);
}

#[test]
fn a_list_value_is_one_element_of_a_list() {
    let chart = Chart::builder()
        .root(Plot::cartesian(
            CartesianPlot::builder().layers([CartesianLayer::symbol(
                SymbolMark::builder()
                    .x(Encoding::scaled_column("a").scale(
                        ScaleSpec::linear(LinearScale::builder())
                            .domain(Domain::explicit([0.0, 10.0])),
                    ))
                    .y(Encoding::scaled_column("b")),
            )
            .table(points())]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    let spec = symbols(&chart).x.as_ref().unwrap().scale.as_ref().unwrap();
    let Some(domain::Domain::Explicit(value)) = spec.domain.as_ref().unwrap().domain else {
        panic!("the domain is explicit")
    };
    let ends = chart.list_elements(&value);
    assert_eq!(ends.as_primitive::<Float64Type>().values(), &[0.0, 10.0]);
}

#[test]
fn a_value_of_any_arrow_type_is_a_scalar_or_a_list() {
    let dates = || Arc::new(Date32Array::from(vec![0, 10])) as ArrayRef;
    let fit = IntervalFit::builder().empty(Value::list(dates()));
    // A value gets its reference only in a chart.
    invalid(fit.build(), "empty", "builds only in a chart");
    // A scalar is one element.
    invalid(
        IntervalFit::builder().empty(Value::scalar(dates())).build(),
        "empty",
        "this one has 2",
    );
}

#[test]
fn an_error_of_a_setter_names_its_path_in_the_chart() {
    let pinned = ScaleSpec::builder().domain(Domain::explicit(Value::scalar(Arc::new(
        Float64Array::from(vec![1.0, 2.0]),
    ))));
    invalid(
        scatter(SymbolMark::builder().size(Encoding::scaled_column("a").scale(pinned))).build(),
        "root.plot.cartesian.layers[0].symbol.size.scale.domain.explicit",
        "one element",
    );
}

#[test]
fn a_chart_takes_the_version_and_infers_only_when_asked() {
    // Without inference, a scaled column needs its scale.
    invalid(
        scatter(SymbolMark::builder()).build(),
        "root.plot.cartesian.layers[0].mark.symbol.x",
        "goes through the channel's scale",
    );
    let chart = scatter(SymbolMark::builder())
        .infer_scales()
        .build()
        .unwrap();
    assert_eq!(chart.message().version, FORMAT_VERSION);
    let x = symbols(&chart).x.as_ref().unwrap().scale.as_ref().unwrap();
    assert!(matches!(x.r#type, Some(scale_spec::Type::Linear(_))));
}

#[test]
fn a_conditional_value_takes_its_predicate_and_value() {
    let conditional = proto::ConditionalValue::scaled_constant(3.0)
        .predicate(Predicate::selected(Selected::builder().selection("brush")))
        .build();
    invalid(conditional, "scaled_constant", "builds only in a chart");
    let plain = proto::ConditionalValue::column("c").build().unwrap();
    assert_eq!(
        plain.value,
        Some(conditional_value::Value::Column("c".to_string()))
    );
}

#[test]
fn a_replaced_value_leaves_no_trace_in_the_chart() {
    let replaced = scatter(SymbolMark::builder().color(Encoding::constant("red").constant("blue")))
        .infer_scales()
        .build()
        .unwrap();
    let once = scatter(SymbolMark::builder().color(Encoding::constant("blue")))
        .infer_scales()
        .build()
        .unwrap();
    assert_eq!(replaced, once);
}
