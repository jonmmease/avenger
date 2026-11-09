//! The arguments of the constructors of avenger-chart through avenger-sql: each kind of parameter
//! builds the fragment that its builder builds, an argument of a oneof picks its member,
//! conversions reach messages through chains, and the errors of a constant call fail at planning.

mod common;

use arrow::array::AsArray;
use avenger_chart::{prelude::*, proto::Fragment};
use prost::Message;

/// The fragment of the one value of `SELECT <expression>`.
async fn fragment(expression: &str) -> Fragment {
    let ctx = common::session();
    let batches = avenger_sql::sql(&ctx, &format!("SELECT {expression} AS value"))
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let bytes = batches[0].column(0).as_binary_view().value(0);
    Fragment::decode(bytes).unwrap()
}

/// Fails unless `expression` builds the fragment `expected`.
async fn builds(expression: &str, expected: Fragment) {
    let actual = fragment(expression).await;
    assert_eq!(
        actual.encode_to_vec(),
        expected.encode_to_vec(),
        "{expression}: {actual:?} and {expected:?}"
    );
}

/// The planning error of `SELECT <expression>`.
async fn error(expression: &str) -> String {
    let ctx = common::session();
    let planned = match avenger_sql::sql(&ctx, &format!("SELECT {expression}")).await {
        Ok(frame) => frame.into_optimized_plan().map(|_| ()),
        Err(error) => Err(error),
    };
    planned.expect_err("the call fails to plan").to_string()
}

#[tokio::test]
async fn each_kind_of_parameter_builds_the_fragment_of_its_builder() {
    // A Boolean, a number, a string, an enum, and an array.
    builds(
        "axis(show_grid => TRUE, angle => 30, domain_color => 'gray', side => 'left', classes => ['a', 'b'])",
        Axis::builder()
            .show_grid(true)
            .angle(30.0)
            .domain_color("gray")
            .side(Side::Left)
            .classes(["a", "b"])
            .to_fragment()
            .unwrap(),
    )
    .await;
    // A message by a fragment and by rule A, and a repeated message.
    builds(
        "axis(title => title('Mass', subtitle => 'grams'), levels => [nested_axis_level(show_labels => FALSE)])",
        Axis::builder()
            .title(Title::new("Mass").subtitle("grams"))
            .levels([NestedAxisLevel::builder().show_labels(false)])
            .to_fragment()
            .unwrap(),
    )
    .await;
    builds(
        "axis(title => 'Mass')",
        Axis::builder().title("Mass").to_fragment().unwrap(),
    )
    .await;
    // A `ValueRef` and a required content first, by position.
    builds(
        "domain_pin(0, 'mid')",
        DomainPin::new(0i64, PinPosition::Mid)
            .to_fragment()
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn two_orders_of_arguments_give_equal_bytes() {
    let first = fragment("interval_fit(zero => FALSE, empty => [0, 10])").await;
    let second = fragment("interval_fit(empty => [0, 10], zero => FALSE)").await;
    assert_eq!(first.encode_to_vec(), second.encode_to_vec());
}

#[tokio::test]
async fn an_argument_picks_the_member_of_its_oneof() {
    // A member's name, and a fragment of a member's message.
    builds(
        "scale(type => 'log')",
        Scale::log(LogScale::builder()).to_fragment().unwrap(),
    )
    .await;
    builds(
        "scale(type => log_scale(base => 2))",
        Scale::log(LogScale::builder().base(2.0))
            .to_fragment()
            .unwrap(),
    )
    .await;
    // A value of a member's type, where an integer sets a floating-point member.
    builds(
        "interval_fit(nice => 5)",
        IntervalFit::builder()
            .tick_count(5.0)
            .to_fragment()
            .unwrap(),
    )
    .await;
    builds(
        "interval_fit(nice => TRUE)",
        IntervalFit::builder().enabled(true).to_fragment().unwrap(),
    )
    .await;
    builds(
        "axis(label_rotation => 45)",
        Axis::builder().label_angle(45.0).to_fragment().unwrap(),
    )
    .await;
    builds(
        "axis(label_rotation => 'radial')",
        Axis::builder()
            .label_orient(PolarTextOrient::Radial)
            .to_fragment()
            .unwrap(),
    )
    .await;
    // A string member before a `ValueRef` member, which takes the rest.
    builds(
        "scale_range(range => 'viridis')",
        Range::palette("viridis").to_fragment().unwrap(),
    )
    .await;
    builds(
        "scale_range(range => ['#e45756', '#4c78a8'])",
        Range::values(["#e45756", "#4c78a8"]).to_fragment().unwrap(),
    )
    .await;
}

#[tokio::test]
async fn conversions_reach_a_message_through_a_chain() {
    // Rule O: a constant sets the explicit domain, and a string the palette of a range.
    builds(
        "scale(domain => [0, 100])",
        Scale::builder()
            .domain(Domain::explicit([0i64, 100]))
            .to_fragment()
            .unwrap(),
    )
    .await;
    builds(
        "scale(range => 'viridis')",
        Scale::builder()
            .range(Range::palette("viridis"))
            .to_fragment()
            .unwrap(),
    )
    .await;
    // A fit reaches the domain of a scale through its data domain.
    builds(
        "scale(domain => interval_fit(zero => FALSE))",
        Scale::builder()
            .domain(DataDomain::builder().fit(IntervalFit::builder().zero(false)))
            .to_fragment()
            .unwrap(),
    )
    .await;
    // Rule O over the leading oneof of a geo plot's options.
    builds(
        "geo(surface => spherical(projection => 'mercator'))",
        Geo::spherical(Spherical::mercator()).to_fragment().unwrap(),
    )
    .await;
}

#[tokio::test]
async fn errors_of_a_constant_call_fail_at_planning() {
    for (expression, expected) in [
        (
            "axis(side => 'up')",
            "axis(): `side` takes bottom, top, left, right, not 'up'",
        ),
        ("scale(type => 'cubic')", "`type` takes one of linear"),
        ("scale(type => axis())", "`type` takes"),
        (
            "axis(title => scale())",
            "`title` takes a string, or a `Title`, not a `Scale`",
        ),
        // A name picks only a member without required content.
        ("scale(type => 'custom')", "not `custom`"),
        (
            "geo(surface => 'spherical')",
            "takes one of planar, not `spherical`",
        ),
        (
            "scale(type => 5)",
            "`type` takes a name (linear, log, pow, symlog, time, band, point, nested_band, ordinal, quantile, quantize, threshold), a `BandScale`",
        ),
        ("title()", "`text` is required"),
        (
            "axis(show_grid => 'yes')",
            "`show_grid` takes a Boolean, not a string",
        ),
        ("cartesian(1, 2)", "cartesian() takes 1 arguments at most"),
    ] {
        let message = error(expression).await;
        assert!(message.contains(expected), "{expression}: {message}");
    }
}
