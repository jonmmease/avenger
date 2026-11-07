//! The generated builders of the messages: each rule of their generation on a real message, and the
//! building of a chart.

mod support;

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, AsArray, Date32Array, Float64Array},
    datatypes::{DataType, Float64Type},
};
use avenger_chart::{
    prelude::*,
    proto::{
        cartesian_layer, conditional_value, domain, domain_fit, encoding, legend_wrap, node, plot,
        predicate, scale,
    },
    FORMAT_VERSION,
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

/// The symbols of the first layer of `chart`.
fn symbols(chart: &Chart) -> &SymbolMark {
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
    let encoding = Encoding::scaled_field("a")
        .direct_field("b")
        .build()
        .unwrap();
    assert_eq!(
        encoding.kind,
        Some(encoding::Kind::DirectField("b".to_string()))
    );
    // A constructor is the member's setter on an empty builder.
    assert_eq!(
        Encoding::scaled_field("a").build().unwrap(),
        Encoding::builder().scaled_field("a").build().unwrap()
    );
}

#[test]
fn a_wrapper_converts_from_its_members() {
    let spec = Scale::builder()
        .domain(DataDomain::builder().fit(IntervalFit::builder().zero(true)))
        .build()
        .unwrap();
    let Some(Domain {
        domain: Some(domain::Domain::Data(data)),
    }) = spec.domain
    else {
        panic!("the domain is a data domain")
    };
    let Some(DomainFit {
        fit: Some(domain_fit::Fit::Interval(fit)),
    }) = data.fit
    else {
        panic!("the fit is an interval fit")
    };
    assert_eq!(fit.zero, Some(true));
    // A message converts as its builder does.
    let domain = DomainBuilder::from(DataDomain::default()).build().unwrap();
    assert!(matches!(domain.domain, Some(domain::Domain::Data(_))));
}

#[test]
fn a_member_without_fields_takes_no_argument() {
    let spec = Scale::builder().quantile().build().unwrap();
    assert!(matches!(spec.r#type, Some(scale::Type::Quantile(_))));
    assert!(matches!(
        LegendWrap::flow().build().unwrap().wrap,
        Some(legend_wrap::Wrap::Flow(_))
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
    let bar = BarMarkBuilder::from(message.clone()).build().unwrap();
    assert_eq!(bar, message);
}

#[test]
fn values_take_the_arrow_type_of_their_rust_type() {
    let layer = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_field("a"))
            .y(Encoding::scaled_field("b"))
            .color(Encoding::direct_value("#e45756"))
            .size(Encoding::direct_value(5)),
    )
    .table(points());
    let chart = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([layer])))
        .build()
        .unwrap();
    let mark = symbols(&chart);
    let value = |encoding: &Option<Encoding>| match &encoding.as_ref().unwrap().kind {
        Some(encoding::Kind::DirectValue(value)) => chart.value(value).clone(),
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
                    .x(Encoding::scaled_field("a").scale(
                        Scale::linear(LinearScale::builder()).domain(Domain::explicit([0.0, 10.0])),
                    ))
                    .y(Encoding::scaled_field("b")),
            )
            .table(points())]),
        ))
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
    // An explicit domain of a scalar of two elements.
    let size = Encoding::scaled_field("a").scale(Scale::builder().domain(Domain::explicit(
        Value::scalar(Arc::new(Float64Array::from(vec![1.0, 2.0]))),
    )));
    let layer = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_field("a"))
            .y(Encoding::scaled_field("b"))
            .size(size),
    )
    .table(points());
    invalid(
        Chart::builder()
            .root(Plot::cartesian(CartesianPlot::builder().layers([layer])))
            .build(),
        "root.plot.cartesian.layers[0].symbol.size.scale.domain.explicit",
        "one element",
    );
}

#[test]
fn a_chart_takes_the_version_and_its_reader_infers_scales() {
    let scatter = || {
        let symbols = CartesianLayer::symbol(
            SymbolMark::builder()
                .x(Encoding::scaled_field("a"))
                .y(Encoding::scaled_field("b")),
        )
        .table(points());
        Chart::builder().root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
    };
    let chart = scatter().build().unwrap();
    assert_eq!(chart.message().version, FORMAT_VERSION);
    let x = symbols(&chart).x.as_ref().unwrap().scale.as_ref().unwrap();
    assert!(matches!(x.r#type, Some(scale::Type::Linear(_))));
}

#[test]
fn a_conditional_value_takes_its_predicate_and_value() {
    let conditional = ConditionalValue::scaled_value(3.0)
        .predicate(Predicate::selected(Selected::builder().selection("brush")))
        .build();
    invalid(conditional, "scaled_value", "builds only in a chart");
    let plain = ConditionalValue::direct_field("c").build().unwrap();
    assert_eq!(
        plain.kind,
        Some(conditional_value::Kind::DirectField("c".to_string()))
    );
}

#[test]
fn a_replaced_value_leaves_no_trace_in_the_chart() {
    let colored = |color: EncodingBuilder| {
        let symbols = CartesianLayer::symbol(
            SymbolMark::builder()
                .x(Encoding::scaled_field("a"))
                .y(Encoding::scaled_field("b"))
                .color(color),
        )
        .table(points());
        Chart::builder()
            .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
            .build()
            .unwrap()
    };
    let replaced = colored(Encoding::direct_value("red").direct_value("blue"));
    let once = colored(Encoding::direct_value("blue"));
    assert_eq!(replaced, once);
}
