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
    let concat = Concat::new(Direction::Row)
        .classes(["a", "b"])
        .classes(["c"])
        .build()
        .unwrap();
    assert_eq!(concat.classes, ["a", "b", "c"]);
}

#[test]
fn a_oneof_member_replaces_the_member_that_is_set() {
    let spec = Scale::linear(LinearScale::builder())
        .log(LogScale::builder())
        .build()
        .unwrap();
    assert!(matches!(spec.r#type, Some(scale::Type::Log(_))));
    // A constructor is the member's setter on an empty builder.
    assert_eq!(
        Scale::log(LogScale::builder()).build().unwrap(),
        Scale::builder().log(LogScale::builder()).build().unwrap()
    );
}

#[test]
fn constructors_take_the_required_content() {
    // Required fields, in the order of their numbers.
    let selection = Selection::new("brush", SelectionResolution::Global)
        .build()
        .unwrap();
    assert_eq!(selection.name, "brush");
    assert_eq!(selection.resolution, SelectionResolution::Global as i32);
    // The member of a required oneof comes after the required fields.
    let conditional = ConditionalValue::direct_field(Predicate::selected("brush"), "c")
        .build()
        .unwrap();
    assert_eq!(
        conditional.kind,
        Some(conditional_value::Kind::DirectField("c".to_string()))
    );
    assert!(conditional.predicate.is_some());
    // The members of a oneof that is not required take the required fields first.
    let select = Select::points("brush", PointSelect::builder().keys(["id"]))
        .build()
        .unwrap();
    assert_eq!(select.selection, "brush");
    // A message without required content has `builder()`.
    assert_eq!(BarMark::builder().build().unwrap(), BarMark::default());
}

#[test]
fn a_message_of_one_required_field_converts_from_it() {
    // A string.
    let custom = Scale::custom("warp").build().unwrap();
    let Some(scale::Type::Custom(custom)) = custom.r#type else {
        panic!("the scale is custom")
    };
    assert_eq!(custom.r#type, "warp");
    let Some(predicate::Predicate::Selected(selected)) =
        Predicate::selected("brush").build().unwrap().predicate
    else {
        panic!("the predicate is a selection")
    };
    assert_eq!(selected.selection, "brush");
    // The items of a repeated field.
    let layer = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"));
    let chart = Chart::builder(Plot::cartesian([layer])).build().unwrap();
    assert!(symbols(&chart).x.is_some());
    // An enum.
    let concat = ConcatBuilder::from(Direction::Column).build().unwrap();
    assert_eq!(concat.direction, Direction::Column as i32);
}

#[test]
fn a_scale_converts_from_its_types_and_its_lifted_fields() {
    // Rule B: the members of the scale's type.
    assert!(matches!(
        ScaleBuilder::from(BandScale::builder())
            .build()
            .unwrap()
            .r#type,
        Some(scale::Type::Band(_))
    ));
    // Rule F: the domain, the range, the axis, and the legend.
    let spec = ScaleBuilder::from(Domain::explicit([0.0, 1.0]));
    invalid(spec.build(), "domain.explicit", "builds only in a chart");
    let spec = ScaleBuilder::from(Axis::builder().show(false))
        .build()
        .unwrap();
    assert_eq!(spec.axis.unwrap().show, Some(false));
    let spec = ScaleBuilder::from(Legend::builder().form(LegendForm::Colorbar))
        .build()
        .unwrap();
    assert_eq!(spec.legend.unwrap().form, LegendForm::Colorbar as i32);
    let spec = ScaleBuilder::from(Range::builder().reverse(true))
        .build()
        .unwrap();
    assert!(spec.range.unwrap().reverse);
}

#[test]
fn an_encoding_takes_strings_numbers_and_bools_by_its_channel() {
    let mark = TextMark::new("label").x("a").y("b").build().unwrap();
    let kind = |encoding: &Option<Encoding>| encoding.as_ref().unwrap().kind.clone().unwrap();
    // A string is a scaled field on a channel with a scale, and a direct one on `text`.
    assert_eq!(kind(&mark.x), encoding::Kind::ScaledField("a".to_string()));
    assert_eq!(
        kind(&mark.text),
        encoding::Kind::DirectField("label".to_string())
    );
    // A number off the positions is a direct value.
    let layer = CartesianLayer::symbol(
        points(),
        SymbolMark::builder().x("a").y("b").size(100.0).opacity(1),
    );
    let chart = Chart::builder(Plot::cartesian([layer])).build().unwrap();
    let mark = symbols(&chart);
    assert!(matches!(kind(&mark.size), encoding::Kind::DirectValue(_)));
    assert!(matches!(
        kind(&mark.opacity),
        encoding::Kind::DirectValue(_)
    ));
    // A number on a position with a scale is a scaled value, a data value such as the place of a
    // reference line.
    let rule = CartesianLayer::rule(points(), RuleMark::builder().x(2.5));
    let chart = Chart::builder(Plot::cartesian([rule])).build().unwrap();
    let Some(node::Node::Plot(plot)) = &chart.root().node else {
        panic!("the root is a plot")
    };
    let Some(plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("the plot is Cartesian")
    };
    let Some(cartesian_layer::Mark::Rule(rule)) = &cartesian.layers[0].mark else {
        panic!("the layer draws a rule")
    };
    assert!(matches!(kind(&rule.x), encoding::Kind::ScaledValue(_)));
    // A second channel follows its partner's scale, so a string on it is scaled.
    let rule = RuleMark::builder().x("a").x2("b").build().unwrap();
    assert_eq!(kind(&rule.x2), encoding::Kind::ScaledField("b".to_string()));
    // A longitude takes no scale, so a string on it is direct.
    let geo = GeoSymbolMark::builder().longitude("lon").build().unwrap();
    assert_eq!(
        kind(&geo.longitude),
        encoding::Kind::DirectField("lon".to_string())
    );
}

#[test]
fn helpers_hide_axes_and_legends() {
    let x = Encoding::scaled_field("a").no_axis().build().unwrap();
    assert_eq!(x.scale.unwrap().axis.unwrap().show, Some(false));
    let color = Encoding::scaled_field("c").no_legend().build().unwrap();
    assert_eq!(color.scale.unwrap().legend.unwrap().show, Some(false));
    let spec = Scale::builder().no_axis().no_legend().build().unwrap();
    assert_eq!(spec.axis.unwrap().show, Some(false));
    assert_eq!(spec.legend.unwrap().show, Some(false));

    let layer = CartesianLayer::symbol(
        points(),
        SymbolMark::builder().x("a").y("b").color("a").size(5.0),
    );
    let chart = Chart::builder(Plot::cartesian(CartesianPlot::new([layer]).no_guides()))
        .build()
        .unwrap();
    let mark = symbols(&chart);
    let spec = |encoding: &Option<Encoding>| encoding.as_ref().unwrap().scale.clone().unwrap();
    assert_eq!(spec(&mark.x).axis.unwrap().show, Some(false));
    assert_eq!(spec(&mark.y).axis.unwrap().show, Some(false));
    assert_eq!(spec(&mark.color).legend.unwrap().show, Some(false));
    // A direct value has no scale.
    assert!(mark.size.as_ref().unwrap().scale.is_none());
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
    let selected = || Predicate::selected("brush");
    let all = Predicate::all([selected(), selected()]).build().unwrap();
    let Some(predicate::Predicate::All(list)) = all.predicate else {
        panic!("the predicate is a list of all")
    };
    assert_eq!(list.predicates.len(), 2);
}

#[test]
fn a_keyword_field_is_a_raw_identifier_in_a_constructor() {
    let custom = CustomScale::new("warp").build().unwrap();
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
        points(),
        SymbolMark::builder()
            .x("a")
            .y("b")
            .color(Encoding::direct_value("#e45756"))
            .size(5),
    );
    let chart = Chart::builder(Plot::cartesian([layer])).build().unwrap();
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
    let chart = Chart::builder(Plot::cartesian([CartesianLayer::symbol(
        points(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("a")
                .scale(Scale::linear(LinearScale::builder()).domain(Domain::explicit([0.0, 10.0]))))
            .y("b"),
    )]))
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
    let size = Encoding::scaled_field("a").scale(Domain::explicit(Value::scalar(Arc::new(
        Float64Array::from(vec![1.0, 2.0]),
    ))));
    let layer = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b").size(size));
    invalid(
        Chart::builder(Plot::cartesian([layer])).build(),
        "root.plot.cartesian.layers[0].symbol.size.scale.domain.explicit",
        "one element",
    );
}

#[test]
fn a_chart_takes_the_version_and_its_reader_infers_scales() {
    let scatter = || {
        let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"));
        Chart::builder(Plot::cartesian([symbols]))
    };
    let chart = scatter().build().unwrap();
    assert_eq!(chart.message().version, FORMAT_VERSION);
    let x = symbols(&chart).x.as_ref().unwrap().scale.as_ref().unwrap();
    assert!(matches!(x.r#type, Some(scale::Type::Linear(_))));
}

#[test]
fn a_conditional_value_takes_its_predicate_and_value() {
    let conditional = ConditionalValue::scaled_value(Predicate::selected("brush"), 3.0).build();
    invalid(conditional, "scaled_value", "builds only in a chart");
}

#[test]
fn a_replaced_value_leaves_no_trace_in_the_chart() {
    let ticked = |ticks: TicksBuilder| {
        let x = Encoding::scaled_field("a").scale(Axis::builder().ticks(ticks));
        let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x(x).y("b"));
        Chart::builder(Plot::cartesian([symbols])).build().unwrap()
    };
    let replaced = ticked(Ticks::values([1.0, 2.0]).count(3.0));
    let once = ticked(Ticks::count(3.0));
    assert_eq!(replaced, once);
}
