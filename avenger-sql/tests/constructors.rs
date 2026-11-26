//! The arguments of the constructors of avenger-chart through avenger-sql: a constructor is valid
//! only in an argument of a chart function, where the argument's parameter names it, each kind of
//! parameter builds the fragment that its builder builds, an argument of a oneof picks its member,
//! a constructor one level down fills the parameter's own constructor, and the errors of a
//! constant call fail at planning at their argument.

mod common;

use arrow::array::{Array, AsArray};
use avenger_chart::{prelude::*, proto::Fragment};
use datafusion::{
    common::{
        tree_node::{TreeNode, TreeNodeRecursion},
        ScalarValue,
    },
    logical_expr::{Expr, ExprSchemable, LogicalPlan},
};
use prost::Message;

/// The channel whose function takes the option `place` of an encoding: `color` for a legend, and
/// `x` for another option.
fn channel_of(place: &str) -> &'static str {
    match place {
        "legend" => "color",
        _ => "x",
    }
}

/// The plan of a symbol call whose channel of `place` is
/// `<channel> => <channel>(one, <place> => <expression>)`, optimized or not.
async fn encoding_plan(
    place: &str,
    expression: &str,
    optimized: bool,
) -> Result<LogicalPlan, String> {
    let ctx = common::session();
    let channel = channel_of(place);
    let others = match channel {
        "x" => "y => one",
        _ => "x => one, y => one",
    };
    let query = format!(
        "FROM one |> AGGREGATE mark_symbol({channel} => {channel}(one, {place} => {expression}), \
         {others}) AS value"
    );
    let frame = avenger_sql::sql(&ctx, &query)
        .await
        .map_err(|error| error.to_string())?;
    match optimized {
        true => frame
            .into_optimized_plan()
            .map_err(|error| error.to_string()),
        false => Ok(frame.into_unoptimized_plan()),
    }
}

/// The calls of the function `name` in `plan`.
fn calls(plan: &LogicalPlan, name: &str) -> Vec<Vec<Expr>> {
    let mut found = Vec::new();
    plan.apply(|node| {
        node.apply_expressions(|expr| {
            expr.apply(|expr| {
                if let Expr::ScalarFunction(call) = expr {
                    if call.func.name() == name {
                        found.push(call.args.clone());
                    }
                }
                Ok(TreeNodeRecursion::Continue)
            })
        })
    })
    .unwrap();
    found
}

/// `expr`, a constant, folded by DataFusion's simplifier, as the binding of a mark call reads it.
fn folded(expr: &Expr) -> Expr {
    let simplifier = datafusion::optimizer::simplify_expressions::ExprSimplifier::new(
        datafusion::logical_expr::simplify::SimplifyContext::default(),
    );
    simplifier.simplify(expr.clone()).unwrap()
}

/// The fragment that `expression` builds as the option `place` of an encoding, through the
/// function of a channel that takes it.
async fn option(place: &str, expression: &str) -> Fragment {
    // The planning checks the call, and the options of the unbound call fold.
    encoding_plan(place, expression, true).await.unwrap();
    let plan = encoding_plan(place, expression, false).await.unwrap();
    let calls = calls(&plan, channel_of(place));
    let args = calls.first().expect("a call of the channel function");
    let bytes = (args.iter().skip(1).map(folded))
        .find_map(|arg| match arg {
            Expr::Literal(ScalarValue::BinaryView(Some(bytes)), _) => Some(bytes.clone()),
            _ => None,
        })
        .expect("a fragment");
    Fragment::decode(bytes.as_slice()).unwrap()
}

/// Fails unless `expression` builds the fragment `expected` as the option `place` of an encoding.
async fn builds(place: &str, expression: &str, expected: Fragment) {
    let actual = option(place, expression).await;
    assert_eq!(
        actual.encode_to_vec(),
        expected.encode_to_vec(),
        "{place} => {expression}: {actual:?} and {expected:?}"
    );
}

/// The planning error of `expression` as the option `place` of an encoding.
async fn error(place: &str, expression: &str) -> String {
    encoding_plan(place, expression, true)
        .await
        .expect_err("the call fails to plan")
}

/// The planning error of `query`.
async fn query_error(query: &str) -> String {
    let ctx = common::session();
    let planned = match avenger_sql::sql(&ctx, query).await {
        Ok(frame) => frame.into_optimized_plan().map(|_| ()),
        Err(error) => Err(error),
    };
    planned.expect_err("the query fails to plan").to_string()
}

#[tokio::test]
async fn each_kind_of_parameter_builds_the_fragment_of_its_builder() {
    // A Boolean, a number, a string, an enum, and an array.
    builds(
        "axis",
        "axis(grid => TRUE, angle => 30, rule_stroke => 'gray', orient => 'left', classes => ['a', 'b'])",
        Axis::builder()
            .grid(true)
            .angle(30.0)
            .rule_stroke("gray")
            .orient(Side::Left)
            .classes(["a", "b"])
            .to_fragment()
            .unwrap(),
    )
    .await;
    // A message by its constructor and by its shorthand, and a repeated message by the
    // constructor of its elements.
    builds(
        "axis",
        "axis(title => title('Mass', subtitle => 'grams'), levels => [level(labels => FALSE)])",
        Axis::builder()
            .title(Title::builder().text("Mass").subtitle("grams"))
            .levels([NestedAxisLevel::builder().labels(false)])
            .to_fragment()
            .unwrap(),
    )
    .await;
    builds(
        "axis",
        "axis(title => 'Mass')",
        Axis::builder().title("Mass").to_fragment().unwrap(),
    )
    .await;
    // A Boolean shows or hides a part, through its lifted `display`, and a string fills the other
    // lifted field of a title or labels.
    builds(
        "axis",
        "axis(title => FALSE, labels => '%.0f', ticks => FALSE, value_placement => value_step(5))",
        Axis::builder()
            .title(false)
            .labels("%.0f")
            .ticks(false)
            .value_step(5.0)
            .to_fragment()
            .unwrap(),
    )
    .await;
    builds(
        "legend",
        "legend(labels => FALSE, columns => 2, background => 'white')",
        Legend::builder()
            .labels(false)
            .columns(2)
            .background("white")
            .to_fragment()
            .unwrap(),
    )
    .await;
    // A pin takes its value by position and its position by name.
    builds(
        "scale",
        "scale(pin => pin(0, position => 'mid'))",
        Scale::builder()
            .pin(DomainPin::new(0i64, PinPosition::Mid))
            .to_fragment()
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn a_value_means_the_same_bare_and_wrapped_in_its_constructor() {
    for (place, bare, wrapped) in [
        (
            "axis",
            "axis(title => 'Mass')",
            "axis(title => title('Mass'))",
        ),
        ("axis", "FALSE", "axis(FALSE)"),
        ("scale", "'log'", "scale('log')"),
        (
            "scale",
            "scale(type => 'log')",
            "scale(type => type('log'))",
        ),
    ] {
        let chart = |value: &str| {
            format!(
                "FROM penguins
                 |> AGGREGATE mark_symbol(x => x(body_mass_g, {place} => {value}), y => species)
                      AS chart"
            )
        };
        assert_eq!(
            chart_of(&chart(bare)).await,
            chart_of(&chart(wrapped)).await,
            "{place} => {bare} and {place} => {wrapped}"
        );
    }
    for (place, bare, wrapped) in [
        (
            "legend",
            "legend(labels => '%.0f')",
            "legend(labels => labels('%.0f'))",
        ),
        (
            "scale",
            "scale(range => 'viridis')",
            "scale(range => range('viridis'))",
        ),
    ] {
        let chart = |value: &str| {
            format!(
                "FROM penguins
                 |> AGGREGATE mark_symbol(
                      x => body_mass_g,
                      y => species,
                      color => color(island, {place} => {value}),
                    ) AS chart"
            )
        };
        assert_eq!(
            chart_of(&chart(bare)).await,
            chart_of(&chart(wrapped)).await,
            "{place} => {bare} and {place} => {wrapped}"
        );
    }
}

/// The one chart of `query`.
async fn chart_of(query: &str) -> Chart {
    chart(query).await
}

#[tokio::test]
async fn two_orders_of_arguments_give_equal_bytes() {
    let first = option("scale", "scale(zero => FALSE, domain_empty => [0, 10])").await;
    let second = option("scale", "scale(domain_empty => [0, 10], zero => FALSE)").await;
    assert_eq!(first.encode_to_vec(), second.encode_to_vec());
}

#[tokio::test]
async fn an_argument_picks_the_member_of_its_oneof() {
    // A member's name, a member's constructor one level down, and the constructor of the oneof.
    builds(
        "scale",
        "scale(type => 'log')",
        Scale::log(LogScale::builder()).to_fragment().unwrap(),
    )
    .await;
    for expression in [
        "scale(type => log(base => 2))",
        "scale(type => type(log => log(base => 2)))",
        "type(log => log(base => 2))",
    ] {
        builds(
            "scale",
            expression,
            Scale::log(LogScale::builder().base(2.0))
                .to_fragment()
                .unwrap(),
        )
        .await;
    }
    // A value of a member's type, where an integer sets a floating-point member.
    builds(
        "scale",
        "scale(nice => 5)",
        Scale::builder().nice_count(5.0).to_fragment().unwrap(),
    )
    .await;
    builds(
        "scale",
        "scale(nice => TRUE)",
        Scale::builder().enabled(true).to_fragment().unwrap(),
    )
    .await;
    builds(
        "axis",
        "axis(labels => labels(angle => 45))",
        Axis::builder()
            .labels(Labels::builder().angle(45.0))
            .to_fragment()
            .unwrap(),
    )
    .await;
    builds(
        "axis",
        "axis(labels => labels(text_orient => 'radial'))",
        Axis::builder()
            .labels(Labels::builder().text_orient(PolarTextOrient::Radial))
            .to_fragment()
            .unwrap(),
    )
    .await;
    // A string member before a `ValueRef` member, which takes the rest.
    builds(
        "scale",
        "scale(range => 'viridis')",
        Scale::builder().scheme("viridis").to_fragment().unwrap(),
    )
    .await;
    builds(
        "scale",
        "scale(range => ['#e45756', '#4c78a8'])",
        Scale::builder()
            .values(["#e45756", "#4c78a8"])
            .to_fragment()
            .unwrap(),
    )
    .await;
    // The first `ValueRef` member of a oneof takes a constant: the explicit domain, not the
    // summary, which the oneof's constructor names.
    builds(
        "scale",
        "scale(domain => [0, 100])",
        Scale::builder()
            .explicit([0i64, 100])
            .to_fragment()
            .unwrap(),
    )
    .await;
    builds(
        "scale",
        "scale(domain => domain(summary => [0, 100]))",
        Scale::builder().summary([0i64, 100]).to_fragment().unwrap(),
    )
    .await;
}

#[tokio::test]
async fn one_name_builds_the_message_of_its_place() {
    // `point` is a point scale in the type of a scale.
    builds(
        "scale",
        "scale(type => point(padding => 0.5))",
        Scale::point(PointScale::builder().padding(0.5))
            .to_fragment()
            .unwrap(),
    )
    .await;
    // `log` is a log scale in the type of a scale, and GoogleSQL's logarithm elsewhere.
    builds(
        "scale",
        "scale(type => log(base => 2))",
        Scale::log(LogScale::builder().base(2.0))
            .to_fragment()
            .unwrap(),
    )
    .await;
    let ctx = common::session();
    let batches = avenger_sql::sql(&ctx, "FROM one |> SELECT LOG(8, 2) AS value")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let value = batches[0]
        .column(0)
        .as_primitive::<arrow::datatypes::Float64Type>()
        .value(0);
    assert!((value - 3.0).abs() < 1e-9, "{value}");
    // In a chart function, a call that GoogleSQL's function takes is GoogleSQL's, beside a
    // constructor of the same name.
    let chart = chart_of(
        "FROM penguins
         |> WHERE body_mass_g IS NOT NULL
         |> AGGREGATE mark_symbol(
              x => x(LOG(body_mass_g), scale => scale(type => log(base => 2))),
              y => species,
            ) AS chart",
    )
    .await;
    let layer = avenger_chart::LayerRef::of(plot(&chart)).remove(0);
    let encoding = layer.mark().unwrap().encoding(Channel::X).unwrap();
    assert!(matches!(
        encoding.scale.as_ref().and_then(|scale| scale.r#type.as_ref()),
        Some(avenger_chart::proto::scale::Type::Log(log)) if log.base == Some(2.0)
    ));
    builds(
        "scale",
        "scale(domain => [1, POW(10, 6)], domain_empty => LOG(100, 10))",
        Scale::builder()
            .explicit([1.0, 1e6])
            .domain_empty(2.0)
            .to_fragment()
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn a_constructor_one_level_down_fills_the_constructor_of_its_parameter() {
    for (expression, scale) in [
        (
            "pin(1000, position => 'low')",
            Scale::builder().pin(DomainPin::new(1000i64, PinPosition::Low)),
        ),
        ("type('log')", Scale::log(LogScale::builder())),
        (
            "navigation('data')",
            Scale::builder().navigation(NavigationLimits::data()),
        ),
    ] {
        builds("scale", expression, scale.to_fragment().unwrap()).await;
    }
    builds(
        "axis",
        "labels(font_size => 9)",
        Axis::builder()
            .labels(Labels::builder().font_size(9.0))
            .to_fragment()
            .unwrap(),
    )
    .await;
    builds(
        "axis",
        "title('Bill length')",
        Axis::builder().title("Bill length").to_fragment().unwrap(),
    )
    .await;
}

#[tokio::test]
async fn the_navigation_limits_take_their_bounds() {
    builds(
        "scale",
        "scale(navigation => 'data')",
        Scale::builder()
            .navigation(NavigationLimits::data())
            .to_fragment()
            .unwrap(),
    )
    .await;
    let extent = Scale::builder()
        .navigation(NavigationLimits::extent(
            Interval::builder().low(0.0).high(10.0),
        ))
        .to_fragment()
        .unwrap();
    for expression in [
        "scale(navigation => navigation(bounds => extent(low => 0, high => 10)))",
        "scale(navigation => bounds(extent => extent(low => 0, high => 10)))",
    ] {
        builds("scale", expression, extent.clone()).await;
    }
}

#[tokio::test]
async fn a_struct_sets_the_named_options_of_a_custom_scale() {
    use avenger_chart::proto::option_value::Value;
    let value = |value| OptionValue { value: Some(value) };
    builds(
        "scale",
        "scale(type => custom('warp', options => STRUCT(2.5 AS exponent)))",
        Scale::custom(CustomScale::new("warp").options([("exponent", value(Value::Float(2.5)))]))
            .to_fragment()
            .unwrap(),
    )
    .await;
    let message = error(
        "scale",
        "scale(type => custom('warp', options => STRUCT(DATE '2024-01-01' AS start)))",
    )
    .await;
    assert!(
        message.contains("`options` takes for the option `start` a Boolean, a number, a string"),
        "{message}"
    );
    let message = error(
        "scale",
        "scale(type => custom('warp', options => STRUCT(1 AS a, 2 AS a)))",
    )
    .await;
    assert!(
        message.contains("`options` takes each option once, and the struct names `a` twice"),
        "{message}"
    );
}

#[tokio::test]
async fn errors_of_a_constant_call_fail_at_planning() {
    for (place, expression, expected) in [
        (
            "axis",
            "axis(orient => 'up')",
            "`orient` takes one of 'bottom', 'top', 'left', 'right'",
        ),
        (
            "scale",
            "scale(type => 'cubic')",
            "`type` takes `type => 'linear' | 'log' | 'pow' | …`",
        ),
        (
            "scale",
            "scale(type => axis())",
            "`type` takes `type => 'linear' | 'log' | 'pow' | …`",
        ),
        ("axis", "axis(title => scale())", "`title` takes"),
        // A name picks only a member without required content.
        ("scale", "scale(type => 'custom')", "not 'custom'"),
        ("scale", "scale(type => 5)", "not a number"),
        (
            "scale",
            "scale(pin => pin())",
            "pin needs its argument `value`",
        ),
        // A value alone is no pin, whose position is required.
        (
            "scale",
            "scale(pin => 1000)",
            "`pin` takes `pin => pin(value: ANY, position => 'low' | 'mid' | 'high')`, not a \
             number",
        ),
        ("axis", "axis(grid => 'yes')", "no form of axis takes"),
        // A constructor takes its shorthand by position and its other parameters by name.
        (
            "scale",
            "scale(pin => pin(1000, 'low'))",
            "pin takes 1 positional argument, and its other parameters by name, as \
             `position => ...`",
        ),
        (
            "scale",
            "scale(type => log(2))",
            "log takes no positional argument, and its other parameters by name, as \
             `base => ...`",
        ),
        // A plain value fills a field of the argument's own message, and a constructor one level
        // down fills a field of a part.
        (
            "axis",
            "'Bill length'",
            "Did you mean `axis => title('Bill length')`?",
        ),
        // A constructor in the wrong argument names the argument that takes it.
        (
            "scale",
            "scale(domain => log())",
            "not `log(...)`. Did you mean `type => log(...)`?",
        ),
        // A constructor two levels down.
        (
            "scale",
            "log(base => 2)",
            "Did you mean `scale => scale(type => log(...))`?",
        ),
        // The error of a call whose arguments are calls of other constructors.
        (
            "scale",
            "scale(navigation => navigation(bounds => 'everywhere'))",
            "`bounds` takes `bounds => 'data' | 'extent'`",
        ),
    ] {
        let message = error(place, expression).await;
        assert!(
            message.contains(expected),
            "{place} => {expression}: {message}"
        );
        assert!(!message.contains("BinaryView"), "{expression}: {message}");
    }
}

#[tokio::test]
async fn a_constructor_is_valid_only_in_an_argument_of_a_chart_function() {
    let message = query_error("FROM one |> SELECT pin(1000, position => 'low') AS a").await;
    assert!(
        message.contains(
            "`pin(...)` builds an option of a chart, so only a chart function's options hold \
             it, as `pin => pin(...)` in `scale(...)`"
        ),
        "{message}"
    );
    let message = query_error("FROM one |> SELECT CONCAT('a', title('b')) AS a").await;
    assert!(
        message.contains("`title(...)` builds an option of a chart"),
        "{message}"
    );
    // A constructor is no function of the library.
    let library = avenger_sql::library::Library::new();
    for constructor in avenger_chart::functions::constructors() {
        if constructor.name.contains('.') {
            continue;
        }
        let function = library.get(constructor.name);
        assert!(
            function.is_none_or(|function| function.family != avenger_sql::library::Family::Constructor),
            "`{}` is a function of the library",
            constructor.name
        );
        assert!(library.constructor(constructor.name).is_some());
    }
}

#[tokio::test]
async fn a_constant_call_folds_to_a_literal() {
    let plan = encoding_plan("axis", "axis(title => 'Mass')", false)
        .await
        .unwrap();
    let args = calls(&plan, "x").remove(0);
    let axis = folded(&args[2]);
    assert!(
        matches!(axis, Expr::Literal(ScalarValue::BinaryView(Some(_)), _)),
        "the call folds: {axis}"
    );
}

#[tokio::test]
async fn a_fragment_has_the_extension_of_fragments() {
    let plan = encoding_plan("axis", "axis(title => 'Mass')", false)
        .await
        .unwrap();
    let args = calls(&plan, "x").remove(0);
    let axis = (args.iter())
        .find(|arg| matches!(arg, Expr::ScalarFunction(call) if call.func.name() == "axis"))
        .expect("a call of the constructor of an axis");
    let (_, field) = axis.to_field(plan.schema()).unwrap();
    let metadata = field.metadata();
    assert_eq!(
        metadata.get("ARROW:extension:name").map(String::as_str),
        Some(avenger_chart::functions::FRAGMENT_EXTENSION_NAME)
    );
    assert_eq!(
        metadata.get("ARROW:extension:metadata").map(String::as_str),
        Some("avenger.chart.v1.Axis")
    );
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
        "FROM penguins |> AGGREGATE mark_bar(x => body_mass_g, y => species, orient => 'horizontal', corner_radius => 2) AS chart",
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
        "FROM penguins |> AGGREGATE mark_polar_bar(theta => species, radius => body_mass_g, coord => polar(inner_radius => 0.5)) AS chart",
    )
    .await;
    let Some(avenger_chart::proto::plot::Coord::Polar(polar)) = &plot(&chart).coord else {
        panic!("a polar plot")
    };
    assert_eq!(polar.options.as_ref().unwrap().inner_radius, Some(0.5));
}

#[tokio::test]
async fn a_geo_mark_takes_the_options_of_its_geo_plot() {
    let chart = chart_of_geo("coord => geo(projection => 'mercator')").await;
    let Some(avenger_chart::proto::plot::Coord::Geo(geo)) = &plot(&chart).coord else {
        panic!("a geo plot")
    };
    let projection = geo.options.as_ref().unwrap().projection.as_ref().unwrap();
    assert_eq!(
        projection.r#type,
        Some(avenger_chart::proto::projection::Type::Builtin(
            ProjectionType::Mercator as i32
        ))
    );
}

#[tokio::test]
async fn the_reader_checks_the_parameters_of_a_projection() {
    for (coord, expected) in [
        (
            "coord => geo(projection => projection(type => 'mercator', coefficient => 2))",
            "`projection.coefficient` applies only to `hammer`, not to `mercator`",
        ),
        (
            "coord => geo(projection => projection(type => 'orthographic', rotate => [96, 0, 0, 1]))",
            "a rotation takes two or three angles",
        ),
    ] {
        let ctx = common::session();
        let query = format!(
            "FROM penguins |> AGGREGATE mark_geo_symbol(longitude => bill_length_mm, latitude => bill_depth_mm, {coord}) AS chart"
        );
        let result = match avenger_sql::sql(&ctx, &query).await {
            Ok(frame) => frame.collect().await.map(|_| ()),
            Err(error) => Err(error),
        };
        let message = result.expect_err("the chart is invalid").to_string();
        assert!(message.contains(expected), "{coord}: {message}");
    }
    let chart = chart_of_geo(
        "coord => geo(projection => projection(type => 'orthographic', rotate => [96, 0]))",
    )
    .await;
    let Some(avenger_chart::proto::plot::Coord::Geo(geo)) = &plot(&chart).coord else {
        panic!("a geo plot")
    };
    let projection = geo.options.as_ref().unwrap().projection.as_ref().unwrap();
    assert_eq!(projection.rotate, [96.0, 0.0]);
}

/// A geo chart of the penguins with the `coord` argument `coord`.
async fn chart_of_geo(coord: &str) -> Chart {
    chart(&format!(
        "FROM penguins |> AGGREGATE mark_geo_symbol(longitude => bill_length_mm, latitude => bill_depth_mm, {coord}) AS chart"
    ))
    .await
}

#[tokio::test]
async fn a_channel_function_takes_the_options_of_its_encoding() {
    let chart = chart(
        "FROM penguins |> AGGREGATE mark_symbol(
           x => x(
             body_mass_g,
             scale => scale(type => 'log'),
             title => 'Mass',
             axis => axis(labels => labels(angle => -45), grid => FALSE),
           ),
           y => y(flipper_length_mm, axis => FALSE),
           color => color(species, legend => legend(orient => 'bottom', columns => 2)),
         ) AS chart",
    )
    .await;
    let layer = avenger_chart::LayerRef::of(plot(&chart)).remove(0);
    let mark = layer.mark().unwrap();
    let encoding = mark.encoding(Channel::X).unwrap();
    assert_eq!(encoding.title.as_deref(), Some("Mass"));
    assert!(matches!(
        encoding
            .scale
            .as_ref()
            .and_then(|scale| scale.r#type.as_ref()),
        Some(avenger_chart::proto::scale::Type::Log(_))
    ));
    // The axis and the legend are options of the encoding, and a Boolean shows or hides a part.
    let axis = encoding.axis.as_ref().unwrap();
    assert_eq!(axis.grid, Some(false));
    assert_eq!(
        axis.labels.as_ref().and_then(|labels| labels.angle),
        Some(-45.0)
    );
    let y = mark.encoding(Channel::Y).unwrap();
    assert_eq!(y.axis.as_ref().unwrap().display, Some(false));
    let legend = mark
        .encoding(Channel::Color)
        .unwrap()
        .legend
        .as_ref()
        .unwrap();
    assert_eq!(legend.columns, Some(2));
    assert_eq!(legend.orient, avenger_chart::proto::Side::Bottom as i32);
}

#[tokio::test]
async fn an_axis_is_an_option_of_a_position_and_a_legend_of_another_channel() {
    // A channel function takes only the options of its channel, so the analysis reports the
    // option at its name, before its value.
    for (query, option, expected) in [
        (
            "FROM penguins
             |> AGGREGATE mark_symbol(
                  x => x(body_mass_g, legend => legend(columns => 2)),
                  y => flipper_length_mm,
                ) AS chart",
            "legend",
            "x has no parameter named `legend`",
        ),
        (
            "FROM penguins
             |> AGGREGATE mark_symbol(
                  x => body_mass_g,
                  y => flipper_length_mm,
                  color => color(species, axis => FALSE),
                ) AS chart",
            "axis",
            "color has no parameter named `axis`: its parameters are `value`, `scale`, `legend`, \
             `title`, and `selection_field`",
        ),
        (
            "FROM penguins
             |> AGGREGATE mark_rule(x => body_mass_g, x2 => x2(flipper_length_mm, axis => FALSE)) AS chart",
            "axis",
            "x2 has no parameter named `axis`",
        ),
        (
            "FROM penguins
             |> AGGREGATE mark_rule(
                  x => body_mass_g,
                  x2 => x2(flipper_length_mm, axis => axis(grid => FALSE)),
                ) AS chart",
            "axis",
            "x2 has no parameter named `axis`",
        ),
    ] {
        let error = match avenger_sql::sql(&common::session(), query).await {
            Ok(_) => panic!("`{query}` plans"),
            Err(datafusion::error::DataFusionError::External(error)) => error,
            Err(error) => panic!("{error} is no ScriptError"),
        };
        let error = (error.downcast_ref::<avenger_sql::ScriptError>()).expect("a ScriptError");
        let [diagnostic] = error.diagnostics.as_slice() else {
            panic!("{query}: one error, not {:?}", error.diagnostics);
        };
        assert!(
            diagnostic.message.contains(expected),
            "{query}: {diagnostic:?}"
        );
        let at = &query[std::ops::Range::<usize>::from(diagnostic.range)];
        assert!(
            at.starts_with(&format!("{option} =>")),
            "{query}: the error is at the option's argument, not at `{at}`"
        );
    }
}

#[tokio::test]
async fn the_options_of_a_mark_are_plan_time_constants() {
    for (query, expected) in [
        (
            "FROM penguins |> AGGREGATE mark_bar(x => species, y => body_mass_g, corner_radius => body_mass_g)",
            "mark_bar() needs a plan-time constant for a property",
        ),
        (
            "FROM penguins |> AGGREGATE mark_symbol(x => x(body_mass_g, title => species), y => flipper_length_mm)",
            "needs a plan-time constant for an option of `x`",
        ),
        (
            "FROM penguins |> AGGREGATE mark_geo_symbol(longitude => bill_length_mm, latitude => bill_depth_mm)",
            "mark_geo_symbol() needs `coord`",
        ),
        // A wrong constructor inside an option fails with its own error, not the mark's.
        (
            "FROM penguins |> AGGREGATE mark_symbol(x => x(body_mass_g, scale => scale(domain => linear())), y => flipper_length_mm)",
            "`domain` takes",
        ),
        (
            "FROM penguins |> AGGREGATE mark_bar(x => species, y => body_mass_g, coord => geo(projection => 'identity'))",
            "`coord` takes `coord => cartesian([aspect_ratio => ANY])`, not `geo(...)`",
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

/// The tools that the argument `tools` of the mark call of `query` builds, before the mark places
/// them.
async fn tools(query: &str) -> Vec<avenger_chart::proto::Tool> {
    let ctx = common::session();
    let plan = avenger_sql::sql(&ctx, query)
        .await
        .unwrap()
        .into_unoptimized_plan();
    let library = avenger_sql::library::Library::new();
    let index = (library.get("mark_symbol").unwrap().forms[0]
        .signature
        .args
        .iter())
    .position(|arg| arg.param == "tools")
    .unwrap();
    let mut found = None;
    plan.apply(|node| {
        node.apply_expressions(|expr| {
            expr.apply(|expr| {
                if let Expr::AggregateFunction(call) = expr {
                    if call.func.name() == "mark_symbol" {
                        found = call.params.args.get(index).cloned();
                    }
                }
                Ok(TreeNodeRecursion::Continue)
            })
        })
    })
    .unwrap();
    let schema = datafusion::common::DFSchema::empty();
    let physical = ctx
        .state()
        .create_physical_expr(found.expect("the tools of symbol"), &schema)
        .unwrap();
    let batch = arrow::array::RecordBatch::try_new_with_options(
        std::sync::Arc::new(arrow::datatypes::Schema::empty()),
        vec![],
        &arrow::array::RecordBatchOptions::new().with_row_count(Some(1)),
    )
    .unwrap();
    let array = physical.evaluate(&batch).unwrap().into_array(1).unwrap();
    let list = array.as_list::<i32>().value(0);
    let items = arrow::compute::cast(&list, &arrow::datatypes::DataType::Binary).unwrap();
    let items = items.as_binary::<i32>();
    (0..items.len())
        .map(|index| message(Fragment::decode(items.value(index)).unwrap()))
        .collect()
}

/// The message of `fragment`.
fn message<M: Message + Default>(fragment: Fragment) -> M {
    M::decode(fragment.message).unwrap()
}

/// A tool of the kind `kind`.
fn tool(kind: avenger_chart::proto::tool::Kind) -> avenger_chart::proto::Tool {
    avenger_chart::proto::Tool { kind: Some(kind) }
}

#[tokio::test]
async fn the_tools_of_a_mark_take_the_members_of_the_choice_of_a_tool() {
    use avenger_chart::proto::tool::Kind;
    let built = tools(
        "DECLARE brush SELECTION;
         FROM penguins
         |> AGGREGATE mark_symbol(
              x => body_mass_g,
              y => flipper_length_mm,
              tools => [
                navigate(modes => ['pan']),
                select(brush, type => interval(channels => ['x']), modes => ['box'], throttle => 100),
                tooltip(
                  nearest => TRUE,
                  max_radius => 20,
                  metric => 'along_x',
                  title => 'species',
                  fields => [field(column => 'island', format => 'x')],
                ),
              ],
            ) AS chart",
    )
    .await;
    let navigate = Navigate::builder().modes(["pan"]).to_fragment().unwrap();
    let select = Select::interval("brush", IntervalSelect::builder().channels([Channel::X]))
        .modes(["box"])
        .throttle(100)
        .to_fragment()
        .unwrap();
    let tooltip = Tooltip::builder()
        .nearest(true)
        .max_radius(20.0)
        .metric(NearestMetric::AlongX)
        .title("species")
        .fields([TooltipField::builder().column("island").format("x")])
        .to_fragment()
        .unwrap();
    assert_eq!(
        built,
        [
            tool(Kind::Navigate(message(navigate))),
            tool(Kind::Select(message(select))),
            tool(Kind::Tooltip(message(tooltip))),
        ]
    );
    // A string names a tool without required content.
    let chart = chart(
        "FROM penguins
         |> AGGREGATE mark_symbol(x => body_mass_g, y => flipper_length_mm, tools => ['navigate']) AS chart",
    )
    .await;
    let navigate = Navigate::builder().to_fragment().unwrap();
    assert_eq!(
        plot(&chart).tools,
        [tool(Kind::Navigate(message(navigate)))]
    );
}

#[tokio::test]
async fn one_name_builds_a_point_scale_and_a_point_selection() {
    use avenger_chart::proto::tool::Kind;
    let built = tools(
        "DECLARE pick SELECTION;
         FROM penguins
         |> AGGREGATE mark_symbol(
              x => x(species, scale => scale(type => point(padding => 0.5))),
              y => flipper_length_mm,
              tools => [select(pick, type => point(keys => ['species'], toggle => TRUE))],
            ) AS chart",
    )
    .await;
    let select = Select::point(
        "pick",
        PointSelect::builder().keys(["species"]).toggle(true),
    )
    .to_fragment()
    .unwrap();
    assert_eq!(built, [tool(Kind::Select(message(select)))]);
    builds(
        "scale",
        "scale(type => point(padding => 0.5))",
        Scale::point(PointScale::builder().padding(0.5))
            .to_fragment()
            .unwrap(),
    )
    .await;
}

/// The projection of the geo plot of `chart`.
fn projection(chart: &Chart) -> avenger_chart::proto::Projection {
    let Some(avenger_chart::proto::plot::Coord::Geo(geo)) = &plot(chart).coord else {
        panic!("a geo plot")
    };
    geo.options.as_ref().unwrap().projection.clone().unwrap()
}

#[tokio::test]
async fn a_projection_takes_arrays_of_numbers_and_shows_its_parts_with_a_boolean() {
    for (coord, expected) in [
        (
            "coord => geo(projection => projection('conic_conformal', parallels => [33, 45], rotate => [96, 0]))",
            Projection::builtin(ProjectionType::ConicConformal)
                .parallels([33.0, 45.0])
                .rotate([96.0, 0.0]),
        ),
        (
            "coord => geo(projection => projection(type => 'mercator', center => [-96, 38], fit => [-125, 24, -66, 50]))",
            Projection::builtin(ProjectionType::Mercator)
                .center([-96.0, 38.0])
                .box_([-125.0, 24.0, -66.0, 50.0]),
        ),
        (
            "coord => geo(projection => projection('identity', reflect_x => TRUE, reflect_y => TRUE))",
            Projection::builtin(ProjectionType::Identity)
                .reflect_x(true)
                .reflect_y(true),
        ),
        (
            "coord => projection('mercator')",
            Projection::builtin(ProjectionType::Mercator),
        ),
    ] {
        let chart = chart_of_geo(coord).await;
        assert_eq!(
            projection(&chart),
            message(expected.to_fragment().unwrap()),
            "{coord}"
        );
    }
    // A Boolean shows a graticule or a sphere with its defaults.
    let chart = chart_of_geo(
        "coord => geo(projection => 'equal_earth', graticule => TRUE, sphere => TRUE)",
    )
    .await;
    let Some(avenger_chart::proto::plot::Coord::Geo(geo)) = &plot(&chart).coord else {
        panic!("a geo plot")
    };
    let expected: Geo = message(
        Geo::new(Projection::builtin(ProjectionType::EqualEarth))
            .graticule(true)
            .sphere(true)
            .to_fragment()
            .unwrap(),
    );
    assert_eq!(geo.options.as_ref().unwrap(), &expected);
}

#[tokio::test]
async fn a_struct_sets_the_named_options_of_a_custom_projection() {
    use avenger_chart::proto::option_value::Value;
    let value = |value| OptionValue { value: Some(value) };
    let chart = chart_of_geo(
        "coord => geo(projection => projection(type => custom('healpix', options => STRUCT(4 AS lobes, 'polar' AS mode, [1, 2.5] AS ratios, ['a', 'b'] AS names, TRUE AS flipped, NULL AS unset))))",
    )
    .await;
    let expected = Projection::custom(CustomProjection::new("healpix").options([
        ("lobes", value(Value::Integer(4))),
        ("mode", value(Value::String("polar".to_string()))),
        (
            "ratios",
            value(Value::Floats(FloatList {
                values: vec![1.0, 2.5],
            })),
        ),
        (
            "names",
            value(Value::Strings(StringList {
                values: vec!["a".to_string(), "b".to_string()],
            })),
        ),
        ("flipped", value(Value::Boolean(true))),
    ]));
    assert_eq!(projection(&chart), message(expected.to_fragment().unwrap()));
}

#[tokio::test]
async fn a_channel_function_resolves_only_as_the_value_of_its_channel() {
    // A channel whose name is also a parameter of a constructor, such as `size` of `resolve` and
    // `color` of `title`, takes its function, and the constructor's parameter takes its value.
    let chart = chart(
        "FROM penguins
         |> AGGREGATE mark_symbol(
              x => x(body_mass_g, axis => axis(title => title('Mass', color => 'gray'))),
              y => species,
              size => size(flipper_length_mm, legend => FALSE),
              angle => angle(bill_depth_mm),
              shape => shape(island),
              opacity => opacity(value(0.5)),
            ) AS chart
            GROUP BY sex
         |> AGGREGATE facet_wrap(chart, sex, resolve => resolve(size => 'independent')) AS chart",
    )
    .await;
    assert!(matches!(
        &chart.root().node,
        Some(avenger_chart::proto::node::Node::Wrap(_))
    ));
    // Outside its channel, a channel function is an error that names where it belongs.
    let message = query_error("FROM one |> SELECT size(one) AS a").await;
    assert!(
        message.contains(
            "`size(...)` sets the channel `size` of a mark, so only the value of that channel \
             holds it, as `size => size(...)` in"
        ),
        "{message}"
    );
    let message = query_error(
        "FROM penguins |> AGGREGATE mark_symbol(x => body_mass_g, y => size(flipper_length_mm)) AS chart",
    )
    .await;
    assert!(
        message.contains("not `size(...)`. Did you mean `size => size(...)`?"),
        "{message}"
    );
    // In an expression of a channel, a channel function is no function.
    let message = query_error(
        "FROM penguins |> AGGREGATE mark_symbol(x => body_mass_g, y => CONCAT(y(species), '')) AS chart",
    )
    .await;
    assert!(
        message.contains("`y(...)` sets the channel `y`"),
        "{message}"
    );
}

#[tokio::test]
async fn datum_takes_a_constant_of_a_script_variable() {
    let chart = chart(
        "DECLARE s DEFAULT 'Adelie';
         FROM penguins
         |> AGGREGATE mark_symbol(x => body_mass_g, y => flipper_length_mm, color => datum(s)) AS chart",
    )
    .await;
    let layer = avenger_chart::LayerRef::of(plot(&chart)).remove(0);
    let color = layer.mark().unwrap().encoding(Channel::Color).unwrap();
    assert!(matches!(
        color.kind,
        Some(avenger_chart::proto::encoding::Kind::ScaledValue(_))
    ));
}

/// The error of reading the charts of `query`.
async fn reading_error(query: &str) -> String {
    let ctx = common::session();
    let result = match avenger_sql::sql(&ctx, query).await {
        Ok(frame) => frame.collect().await.map(|_| ()),
        Err(error) => Err(error),
    };
    result.expect_err("the chart is invalid").to_string()
}

#[tokio::test]
async fn labels_take_the_settings_that_their_guide_draws() {
    let symbol = |x: &str, color: &str| {
        format!(
            "FROM penguins |> AGGREGATE mark_symbol(x => {x}, y => bill_depth_mm{color}) AS chart"
        )
    };
    let polar = |labels: &str| {
        format!(
            "FROM penguins |> AGGREGATE mark_polar_bar(theta => species, radius => radius(body_mass_g, \
             axis => axis(labels => labels({labels})))) AS chart"
        )
    };
    // Each guide's labels take the settings that the guide draws.
    for query in [
        symbol(
            "x(bill_length_mm, axis => axis(labels => labels(angle => -45, text_align => 'end')))",
            "",
        ),
        symbol(
            "bill_length_mm",
            ", color => color(species, legend => legend(labels => labels(margin => 8, angle => 30)))",
        ),
        polar("text_orient => 'radial'"),
    ] {
        common::charts(&query).await;
    }
    // And no others.
    for (query, expected) in [
        (
            symbol("x(bill_length_mm, axis => axis(labels => labels(margin => 4)))", ""),
            "the labels of the axis of a Cartesian plot take no `margin`",
        ),
        (
            symbol(
                "bill_length_mm",
                ", color => color(species, legend => legend(labels => labels(text_align => 'end')))",
            ),
            "the labels of a symbol legend take no `text_align`",
        ),
        (
            polar("angle => 30, text_orient => 'radial'"),
            "`angle` and `text_orient` both turn the labels",
        ),
    ] {
        let message = reading_error(&query).await;
        assert!(message.contains(expected), "{query}: {message}");
    }
}

#[tokio::test]
async fn a_legend_places_its_values_as_an_axis_does() {
    for channel in ["color", "size"] {
        let chart = chart(&format!(
            "FROM penguins |> AGGREGATE mark_symbol(x => bill_length_mm, y => bill_depth_mm, \
             {channel} => {channel}(body_mass_g, legend => legend(value_placement => 3))) AS chart"
        ))
        .await;
        let Some(avenger_chart::proto::plot::Coord::Cartesian(_)) = &plot(&chart).coord else {
            panic!("a Cartesian plot")
        };
        let layer = avenger_chart::LayerRef::of(plot(&chart)).remove(0);
        let channel = match channel {
            "color" => Channel::Color,
            _ => Channel::Size,
        };
        let legend = layer
            .mark()
            .unwrap()
            .encoding(channel)
            .unwrap()
            .legend
            .clone();
        assert_eq!(
            legend.and_then(|legend| legend.value_placement),
            Some(avenger_chart::proto::legend::ValuePlacement::ValueCount(
                3.0
            )),
            "{channel:?}"
        );
    }
}

#[tokio::test]
async fn a_subtitle_is_a_text_or_its_constructor() {
    for (title, color) in [
        ("title('Penguins', subtitle => 'By island')", ""),
        (
            "title('Penguins', subtitle => subtitle('By island', color => 'gray'))",
            "gray",
        ),
    ] {
        let chart = chart(&format!(
            "FROM penguins |> AGGREGATE mark_symbol(x => bill_length_mm, y => bill_depth_mm, \
             title => {title}) AS chart"
        ))
        .await;
        let subtitle = (plot(&chart).title.as_ref())
            .and_then(|title| title.subtitle.clone())
            .unwrap();
        assert_eq!(subtitle.text, "By island");
        assert_eq!(subtitle.color, color);
    }
}
