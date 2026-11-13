//! The arguments of the constructors of avenger-chart through avenger-sql: each kind of parameter
//! builds the fragment that its builder builds, an argument of a oneof picks its member,
//! conversions reach messages through chains, and the errors of a constant call fail at planning.

mod common;

use arrow::array::AsArray;
use avenger_chart::{prelude::*, proto::Fragment};
use prost::Message;

/// The fragment of the one value of `FROM one |> SELECT <expression>`.
async fn fragment(expression: &str) -> Fragment {
    let ctx = common::session();
    let batches = avenger_sql::sql(&ctx, &format!("FROM one |> SELECT {expression} AS value"))
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

/// The planning error of `FROM one |> SELECT <expression>`.
async fn error(expression: &str) -> String {
    let ctx = common::session();
    let planned = match avenger_sql::sql(&ctx, &format!("FROM one |> SELECT {expression}")).await {
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
        "scale_range(range_ => 'viridis')",
        Range::palette("viridis").to_fragment().unwrap(),
    )
    .await;
    builds(
        "scale_range(range_ => ['#e45756', '#4c78a8'])",
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
        "scale(range_ => 'viridis')",
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
            "`side` takes one of 'bottom', 'top', 'left', 'right'",
        ),
        ("scale(type => 'cubic')", "`type` takes one of linear"),
        ("scale(type => axis())", "`type` takes"),
        (
            "axis(title => scale())",
            "`title` takes a string, or `title(...)`, not `scale(...)`",
        ),
        // A name picks only a member without required content.
        ("scale(type => 'custom')", "not `custom`"),
        (
            "geo(surface => 'spherical')",
            "takes one of planar, not `spherical`",
        ),
        (
            "scale(type => 5)",
            "`type` takes a name (linear, log, pow, symlog, time, band, point, nested_band, ordinal, quantile, quantize, threshold), `band_scale(...)`",
        ),
        ("title()", "title needs its argument `text`"),
        (
            "axis(show_grid => 'yes')",
            "no form of axis takes (STRING)",
        ),
        ("cartesian(1, 2)", "cartesian takes 1 positional argument"),
    ] {
        let message = error(expression).await;
        assert!(message.contains(expected), "{expression}: {message}");
    }
}

/// The one chart of `query`.
async fn chart(query: &str) -> Chart {
    let mut charts = common::charts(query).await;
    assert_eq!(charts.len(), 1, "{query}");
    charts.remove(0)
}

/// The root plot of `chart`.
fn plot(chart: &Chart) -> &avenger_chart::proto::Plot {
    match &chart.root().node {
        Some(avenger_chart::proto::node::Node::Plot(plot)) => plot,
        other => panic!("the root is a plot, not {other:?}"),
    }
}

#[tokio::test]
async fn a_property_sets_its_field_of_the_mark() {
    let chart = chart(
        "FROM penguins |> AGGREGATE bar(x => body_mass_g, y => species, orient => 'horizontal', corner_radius => 2) AS chart",
    )
    .await;
    let layer = avenger_chart::LayerRef::of(plot(&chart)).remove(0);
    let avenger_chart::LayerRef::Cartesian(layer) = layer else {
        panic!("a Cartesian layer")
    };
    let Some(avenger_chart::proto::cartesian_layer::Mark::Bar(bar)) = &layer.mark else {
        panic!("a bar")
    };
    assert_eq!(bar.orient(), Orient::Horizontal);
    assert_eq!(bar.corner_radius, Some(2.0));
}

#[tokio::test]
async fn coord_sets_the_options_of_the_plot() {
    let chart = chart(
        "FROM penguins |> AGGREGATE polar_bar(theta => species, radius => body_mass_g, coord => polar(inner_radius => 0.5)) AS chart",
    )
    .await;
    let Some(avenger_chart::proto::plot::Coord::Polar(polar)) = &plot(&chart).coord else {
        panic!("a polar plot")
    };
    assert_eq!(polar.options.as_ref().unwrap().inner_radius, Some(0.5));
}

#[tokio::test]
async fn a_geo_mark_takes_the_options_of_its_geo_plot() {
    let chart = chart(
        "FROM penguins |> AGGREGATE geo_symbol(longitude => bill_length_mm, latitude => bill_depth_mm, coord => geo(surface => spherical(projection => 'mercator'))) AS chart",
    )
    .await;
    let Some(avenger_chart::proto::plot::Coord::Geo(geo)) = &plot(&chart).coord else {
        panic!("a geo plot")
    };
    let options = geo.options.as_ref().unwrap();
    let Some(avenger_chart::proto::geo::Surface::Spherical(spherical)) = &options.surface else {
        panic!("a spherical surface")
    };
    assert!(matches!(
        spherical.projection,
        Some(avenger_chart::proto::spherical::Projection::Mercator(_))
    ));
    // The rule of the leading oneof gives the same options from the spherical surface alone.
    let short = chart_of_geo("coord => spherical(projection => 'mercator')").await;
    assert_eq!(plot(&short), plot(&chart));
}

/// A geo chart of the penguins with the `coord` argument `coord`.
async fn chart_of_geo(coord: &str) -> Chart {
    chart(&format!(
        "FROM penguins |> AGGREGATE geo_symbol(longitude => bill_length_mm, latitude => bill_depth_mm, {coord}) AS chart"
    ))
    .await
}

#[tokio::test]
async fn scaled_takes_the_options_of_its_encoding() {
    let chart = chart(
        "FROM penguins |> AGGREGATE symbol(x => scaled(body_mass_g, scale => scale(type => 'log'), title => 'Mass'), y => flipper_length_mm) AS chart",
    )
    .await;
    let layer = avenger_chart::LayerRef::of(plot(&chart)).remove(0);
    let encoding = layer.mark().unwrap().encoding(Channel::X).unwrap();
    assert_eq!(encoding.title.as_deref(), Some("Mass"));
    assert!(matches!(
        encoding
            .scale
            .as_ref()
            .and_then(|scale| scale.r#type.as_ref()),
        Some(avenger_chart::proto::scale::Type::Log(_))
    ));
}

#[tokio::test]
async fn the_options_of_a_mark_are_plan_time_constants() {
    for (query, expected) in [
        (
            "FROM penguins |> AGGREGATE bar(x => species, y => body_mass_g, corner_radius => body_mass_g)",
            "bar() needs a plan-time constant for a property",
        ),
        (
            "FROM penguins |> AGGREGATE symbol(x => scaled(body_mass_g, title => species), y => flipper_length_mm)",
            "needs a plan-time constant for an option of `x`",
        ),
        (
            "FROM penguins |> AGGREGATE geo_symbol(longitude => bill_length_mm, latitude => bill_depth_mm)",
            "geo_symbol() needs `coord`",
        ),
        (
            "FROM penguins |> AGGREGATE bar(x => species, y => body_mass_g, coord => geo(surface => 'planar'))",
            "`coord` takes `cartesian(...)`, not `geo(...)`",
        ),
    ] {
        let ctx = common::session();
        let planned = match avenger_sql::sql(&ctx, query).await {
            Ok(frame) => frame.into_optimized_plan().map(|_| ()),
            Err(error) => Err(error),
        };
        let message = planned.expect_err("the call fails to plan").to_string();
        assert!(message.contains(expected), "{query}: {message}");
    }
}

#[tokio::test]
async fn a_constant_call_folds_to_a_literal() {
    let ctx = common::session();
    let plan = avenger_sql::sql(&ctx, "FROM one |> SELECT axis(title => 'Mass') AS a")
        .await
        .unwrap()
        .into_optimized_plan()
        .unwrap();
    let display = plan.display_indent().to_string();
    assert!(!display.contains("axis("), "the call folds: {display}");
}

#[test]
fn every_constructor_is_a_function_of_the_library_that_a_call_can_name() {
    let library = avenger_sql::library::Library::new();
    for name in avenger_chart::functions::constructors()
        .iter()
        .map(|c| c.name)
    {
        assert!(library.get(name).is_some(), "`{name}` is in the library");
        // A keyword does not take the name of a constructor.
        let text = format!("FROM one |> SELECT {name}()");
        assert!(avenger_sql_syntax::parse(&text).ok(), "{text}");
    }
}

#[tokio::test]
async fn a_column_of_fragments_has_the_extension_of_fragments() {
    let ctx = common::session();
    let batches = avenger_sql::sql(&ctx, "FROM one |> SELECT axis() AS a")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let schema = batches[0].schema();
    let metadata = schema.field(0).metadata();
    assert_eq!(
        metadata.get("ARROW:extension:name").map(String::as_str),
        Some(avenger_chart::functions::FRAGMENT_EXTENSION_NAME)
    );
    assert_eq!(
        metadata.get("ARROW:extension:metadata").map(String::as_str),
        Some("avenger.chart.v1.Axis")
    );
}
