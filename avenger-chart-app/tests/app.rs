//! A chart app driven by window events: navigation of the declared plots, tooltips, boxes,
//! lassos, and clicks of selections, the changes that the app reports, and updates through its
//! handle.

mod support;

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use arrow::array::{Float64Array, RecordBatch, StringArray};
use avenger_app::app::AvengerApp;
use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartApp, ChartAppState, ChartChange, ChartUpdate, ChartUpdates, Fit, Reports, ScaleIdentity,
};
use avenger_eventstream::window::ElementState;
use avenger_scenegraph::marks::mark::SceneMark;

use support::*;

/// A facet row of the cars of each of `origins`, whose plots share the id `panel`, take the scale
/// `x` for `x`, and declare `tools`.
fn facets(
    origins: &[&str],
    x: impl Fn() -> ScaleBuilder,
    tools: impl Fn() -> Vec<ToolBuilder>,
) -> Chart {
    let panels = Node::partitions(
        &cars_of(origins),
        "origin",
        PartitionOrder::Appearance,
        |rows| {
            let symbols = CartesianLayer::symbol(
                rows,
                SymbolMark::builder()
                    .x(Encoding::scaled_field("weight").scale(x()))
                    .y("mpg"),
            );
            Ok(Plot::cartesian([symbols]).id("panel").tools(tools()))
        },
    )
    .unwrap();
    Chart::builder(
        Concat::new(Direction::Row)
            .facet("origin")
            .resolve([Resolve::channel(Sharing::Independent, Channel::X)])
            .children(panels),
    )
    .build()
    .unwrap()
}

/// A scale of `x` in the share family `weight`.
fn weight_family() -> ScaleBuilder {
    Scale::builder().family("weight")
}

/// A scatter plot `plot` of the cars, with `tools`, in a chart with the modes `pan` and `box` of
/// the group `drag`.
fn scatter(cars: RecordBatch, tools: Vec<ToolBuilder>) -> Chart {
    let color = Encoding::scaled_field("origin").conditional([ConditionalValue::direct_value(
        Predicate::negate(Predicate::selected("brush")),
        "lightgray",
    )]);
    let symbols = CartesianLayer::symbol(
        cars,
        SymbolMark::builder().x("weight").y("mpg").color(color),
    );
    Chart::builder(Plot::cartesian([symbols]).id("plot").tools(tools))
        .selections([Selection::new("brush", SelectionResolution::Union)])
        .modes([
            Mode::new("pan").group("drag"),
            Mode::new("box").group("drag"),
        ])
        .build()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drag_in_one_facet_pans_every_facet_of_its_family() {
    let (mut app, _, _) = app(facets(&["USA", "Japan"], weight_family, navigate_x)).await;
    let fitted = [domains(&app, 0), domains(&app, 1)];
    let from = center(&app, 0);
    send(&mut app, drag(from, [from[0] + 60.0, from[1] + 30.0])).await;

    let panned = [domains(&app, 0), domains(&app, 1)];
    // The facets share `x` through the family, and the navigation moves `x` alone.
    assert_eq!(panned[0][0], panned[1][0]);
    assert!(panned[0][0].0 < fitted[0][0].0, "{panned:?}");
    assert_eq!(panned[0][1], fitted[0][1]);
    assert_eq!(panned[1][1], fitted[1][1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_double_click_returns_the_scales_to_their_fitted_domains() {
    let (mut app, _, _) = app(facets(&["USA", "Japan"], weight_family, navigate_x)).await;
    let fitted = domains(&app, 1);
    let at = center(&app, 1);
    send(&mut app, [moved(at), wheel(2.0)]).await;
    assert_ne!(domains(&app, 1), fitted);

    // The click stands apart from the wheel, so the next press is a double click.
    send(&mut app, double_click(at)).await;
    assert_eq!(domains(&app, 1), fitted);
    assert!(app.state().evaluator().scale_view(1, Channel::X).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_settled_navigation_reports_the_domains_of_its_scales_in_one_change() {
    let (mut app, _, changes) = app(facets(&["USA", "Japan"], weight_family, navigate)).await;
    let from = center(&app, 0);
    send(&mut app, drag(from, [from[0] - 40.0, from[1] + 30.0])).await;

    let changes = changes.lock().unwrap();
    let [ChartChange {
        domains: settled,
        selections,
        settled: true,
        ..
    }] = &changes[..]
    else {
        panic!("one settled change, and not {changes:?}");
    };
    assert!(selections.is_empty());
    let usa = app.state().plots()[0].identity.clone();
    let scales: Vec<&ScaleIdentity> = settled.iter().map(|(scale, _)| scale).collect();
    assert_eq!(
        scales,
        [
            &ScaleIdentity::Family {
                family: "weight".into(),
                channel: Channel::X,
            },
            &ScaleIdentity::Plot {
                plot: usa,
                channel: Channel::Y,
                index: 0,
            },
        ]
    );
    let reported: Vec<(f64, f64)> = (settled.iter())
        .map(|(_, domain)| {
            let domain = domain.as_any().downcast_ref::<Float64Array>().unwrap();
            (domain.value(0), domain.value(1))
        })
        .collect();
    assert_eq!(
        reported,
        domains(&app, 0),
        "the reported domains are the shown ones"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_navigation_limits_of_a_scale_bound_its_view() {
    let limited = || {
        Scale::builder()
            .family("weight")
            .navigation_limits(NavigationLimits::builder().data().min_span(500.0))
    };
    let (mut app, _, _) = app(facets(&["USA"], limited, navigate_x)).await;
    let fitted = domains(&app, 0)[0];
    let at = center(&app, 0);

    // The fitted domain bounds the view, so the view neither zooms out nor pans past it.
    send(&mut app, [moved(at), wheel(-2.0)]).await;
    assert_eq!(domains(&app, 0)[0], fitted);
    send(&mut app, [2.0; 8].map(wheel)).await;
    let (start, end) = domains(&app, 0)[0];
    assert!((end - start - 500.0).abs() < 1e-6, "{start} {end}");
    send(&mut app, drag(at, [at[0] + 4000.0, at[1]])).await;
    assert!((domains(&app, 0)[0].0 - fitted.0).abs() < 1e-6);
}

/// The position of the first row of `cars` in the plot at `plot`.
fn first_car(app: &AvengerApp<ChartAppState>, plot: usize, cars: &RecordBatch) -> [f32; 2] {
    let plot = &app.state().plots()[plot];
    let [x, y] = plot
        .coords
        .pixels([
            &cars.column_by_name("weight").unwrap().slice(0, 1),
            &cars.column_by_name("mpg").unwrap().slice(0, 1),
        ])
        .unwrap();
    [plot.rect[0] + x.value(0), plot.rect[1] + y.value(0)]
}

/// The tooltip scene's marks: the third scene, above the chart and the brushes.
fn tooltip_marks(app: &AvengerApp<ChartAppState>) -> &[SceneMark] {
    &app.scenes()[2].marks
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tooltip_shows_over_the_layers_that_declare_it() {
    // Two layers of one plot, where only the layer `heavy` declares a tooltip.
    let layer = |cars: RecordBatch, id: &str, tools: Vec<LayerToolBuilder>| {
        CartesianLayer::symbol(cars, SymbolMark::builder().x("weight").y("mpg"))
            .id(id)
            .tools(tools)
    };
    let (usa, japan) = (cars_of(&["USA"]), cars_of(&["Japan"]));
    let tooltip = Tooltip::nearest(NearestFind::builder().max_radius(10.0));
    let chart = Chart::builder(
        Plot::cartesian([
            layer(usa.clone(), "heavy", vec![tooltip.into()]),
            layer(japan.clone(), "light", Vec::new()),
        ])
        .id("plot"),
    )
    .build()
    .unwrap();
    let (mut app, _, _) = app(chart).await;
    let light = first_car(&app, 0, &japan);
    send(&mut app, [moved(light)]).await;
    assert!(tooltip_marks(&app).is_empty());
    let heavy = first_car(&app, 0, &usa);
    send(&mut app, [moved(heavy)]).await;
    assert!(!tooltip_marks(&app).is_empty());
}

/// The texts of the tooltip scene.
fn tooltip_texts(app: &AvengerApp<ChartAppState>) -> Vec<String> {
    fn texts(marks: &[SceneMark], out: &mut Vec<String>) {
        for mark in marks {
            match mark {
                SceneMark::Text(text) => out.extend(text.text.as_vec(text.len as usize, None)),
                SceneMark::Group(group) => texts(&group.marks, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    texts(tooltip_marks(app), &mut out);
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tooltip_of_a_lump_shows_its_category_or_the_other_label() {
    // Ports by tons, lumped to the top 2: Rotterdam and Hamburg are kept, and Antwerp and Le Havre
    // are the Other group.
    let fields = arrow::datatypes::Fields::from(vec![
        arrow::datatypes::Field::new("value", arrow::datatypes::DataType::Utf8, true),
        arrow::datatypes::Field::new("other", arrow::datatypes::DataType::Utf8, true),
        arrow::datatypes::Field::new("weight", arrow::datatypes::DataType::Float64, true),
        arrow::datatypes::Field::new("category_count", arrow::datatypes::DataType::Int64, false),
    ]);
    let port = arrow::array::StructArray::new(
        fields,
        vec![
            Arc::new(StringArray::from(vec![
                Some("Rotterdam"),
                Some("Hamburg"),
                None,
                None,
            ])),
            Arc::new(StringArray::from(vec![
                None,
                None,
                Some("Other"),
                Some("Other"),
            ])),
            Arc::new(Float64Array::from(vec![40.0, 30.0, 25.0, 25.0])),
            Arc::new(arrow::array::Int64Array::from(vec![1, 1, 2, 2])),
        ],
        None,
    );
    let ports = RecordBatch::try_from_iter([
        ("port", Arc::new(port) as arrow::array::ArrayRef),
        ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0]))),
    ])
    .unwrap();
    let tooltip = Tooltip::nearest(NearestFind::builder().max_radius(10.0))
        .title(TooltipField::builder().column("port"))
        .fields([TooltipField::builder()
            .column("port")
            .title("lump")
            .labels(Labels::builder().markup("#value (#count, #weight)"))]);
    let symbols = CartesianLayer::symbol(
        ports.clone(),
        SymbolMark::builder().x("x").y("x").color("port"),
    )
    .id("ports")
    .tools([LayerToolBuilder::from(tooltip)]);
    let chart = Chart::builder(Plot::cartesian([symbols]).id("plot"))
        .build()
        .unwrap();
    let (mut app, _, _) = app(chart).await;
    let at = |app: &AvengerApp<ChartAppState>, row: usize| {
        let plot = &app.state().plots()[0];
        let x = ports.column_by_name("x").unwrap().slice(row, 1);
        let [x, y] = plot.coords.pixels([&x, &x]).unwrap();
        [plot.rect[0] + x.value(0), plot.rect[1] + y.value(0)]
    };

    let kept = at(&app, 0);
    send(&mut app, [moved(kept)]).await;
    let texts = tooltip_texts(&app);
    assert!(texts.iter().any(|text| text == "Rotterdam"), "{texts:?}");
    assert!(
        texts
            .iter()
            .any(|text| text.contains("Rotterdam") && text.contains("#(1)")),
        "{texts:?}"
    );

    let other = at(&app, 2);
    send(&mut app, [moved(other)]).await;
    let texts = tooltip_texts(&app);
    assert!(texts.iter().any(|text| text == "Other"), "{texts:?}");
    assert!(
        texts
            .iter()
            .any(|text| text.contains("Other") && text.contains("#(2)") && text.contains("\"25\"")),
        "{texts:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_of_the_other_bar_selects_the_values_outside_the_kept_categories() {
    // The counts of cars by their cylinders lumped to the top 2: 4 and 8 are kept, and 6, 3, and
    // 5 are the Other group.
    let fields = arrow::datatypes::Fields::from(vec![
        arrow::datatypes::Field::new("value", arrow::datatypes::DataType::Int64, true),
        arrow::datatypes::Field::new("other", arrow::datatypes::DataType::Utf8, true),
        arrow::datatypes::Field::new("weight", arrow::datatypes::DataType::Float64, true),
        arrow::datatypes::Field::new("category_count", arrow::datatypes::DataType::Int64, false),
    ]);
    let lumped = arrow::array::StructArray::new(
        fields,
        vec![
            Arc::new(arrow::array::Int64Array::from(vec![Some(4), Some(8), None])),
            Arc::new(StringArray::from(vec![None, None, Some("Other")])),
            Arc::new(Float64Array::from(vec![207.0, 108.0, 91.0])),
            Arc::new(arrow::array::Int64Array::from(vec![1, 1, 3])),
        ],
        None,
    );
    let counts = RecordBatch::try_from_iter([
        ("lumped", Arc::new(lumped) as arrow::array::ArrayRef),
        (
            "cars",
            Arc::new(Float64Array::from(vec![207.0, 108.0, 91.0])),
        ),
    ])
    .unwrap();
    let points = PointSelect::builder()
        .keys([PointKey::new("lumped").source("cylinders")])
        .toggle(true);
    let bars = CartesianLayer::bar(counts.clone(), BarMark::builder().x("lumped").y("cars"));
    let chart = Chart::builder(
        Plot::cartesian([bars])
            .id("plot")
            .tools([Select::points("brush", points)]),
    )
    .selections([Selection::new("brush", SelectionResolution::Union)])
    .build()
    .unwrap();
    let (mut app, _, _) = app(chart).await;
    // The middle of the bar of each row.
    let bar = |app: &AvengerApp<ChartAppState>, row: usize| {
        let plot = &app.state().plots()[0];
        let x = avenger_chart::lump::keys(counts.column(0))
            .unwrap()
            .slice(row, 1);
        let cars = counts
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let y: arrow::array::ArrayRef = Arc::new(Float64Array::from(vec![cars.value(row) / 2.0]));
        let [x, y] = plot.coords.pixels([&x, &y]).unwrap();
        [plot.rect[0] + x.value(0), plot.rect[1] + y.value(0)]
    };
    let set = |app: &AvengerApp<ChartAppState>| match &brush(app)[..] {
        [clause] => match &clause.conditions[..] {
            [avenger_selection::Condition::Set {
                columns,
                values,
                complement,
                ..
            }] => {
                assert_eq!(columns, &["cylinders".to_string()]);
                let numbers = |values: &arrow::array::ArrayRef| {
                    let values = values
                        .as_any()
                        .downcast_ref::<arrow::array::Int64Array>()
                        .unwrap();
                    values.iter().flatten().collect::<Vec<i64>>()
                };
                (numbers(&values[0]), *complement)
            }
            conditions => panic!("a set, and not {conditions:?}"),
        },
        clauses => panic!("one clause, and not {clauses:?}"),
    };

    // A click of the Other bar selects every value outside the kept 4 and 8, their complement.
    let other = bar(&app, 2);
    send(
        &mut app,
        [
            moved(other),
            mouse(ElementState::Pressed),
            mouse(ElementState::Released),
        ],
    )
    .await;
    assert_eq!(set(&app), (vec![4, 8], true));

    // A Shift click of the 4 bar adds it, so the complement leaves out only 8.
    std::thread::sleep(Duration::from_millis(600));
    let four = bar(&app, 0);
    send(&mut app, shift_click(four)).await;
    assert_eq!(set(&app), (vec![8], true));

    // Another Shift click of the Other bar removes the Other group.
    std::thread::sleep(Duration::from_millis(600));
    let other = bar(&app, 2);
    send(&mut app, shift_click(other)).await;
    assert_eq!(set(&app), (vec![4], false));
}

/// Counts of cars by their lumped cylinders: one row of each kept value, of the Other group where
/// `others` names it, and of the NULL category where both are NULL, with `cars` cars each. With
/// `valid`, a row where it is false is a NULL lump, as `'nulls', 'drop'` writes it.
fn lumped_counts(
    values: Vec<Option<i64>>,
    others: Vec<Option<&str>>,
    cars: Vec<f64>,
    valid: Option<Vec<bool>>,
) -> RecordBatch {
    let fields = arrow::datatypes::Fields::from(vec![
        arrow::datatypes::Field::new("value", arrow::datatypes::DataType::Int64, true),
        arrow::datatypes::Field::new("other", arrow::datatypes::DataType::Utf8, true),
        arrow::datatypes::Field::new("weight", arrow::datatypes::DataType::Float64, true),
        arrow::datatypes::Field::new("category_count", arrow::datatypes::DataType::Int64, true),
    ]);
    let counts: Vec<i64> = others
        .iter()
        .map(|other| if other.is_some() { 3 } else { 1 })
        .collect();
    let lumped = arrow::array::StructArray::new(
        fields,
        vec![
            Arc::new(arrow::array::Int64Array::from(values)),
            Arc::new(StringArray::from(others)),
            Arc::new(Float64Array::from(cars.clone())),
            Arc::new(arrow::array::Int64Array::from(counts)),
        ],
        valid.map(Into::into),
    );
    RecordBatch::try_from_iter([
        ("lumped", Arc::new(lumped) as arrow::array::ArrayRef),
        ("cars", Arc::new(Float64Array::from(cars))),
    ])
    .unwrap()
}

/// An app of bars of `counts`, keyed by the column `key` with the source `source` when it has
/// one, with a point selection `brush` that a Shift click toggles.
async fn bars_app(
    counts: &RecordBatch,
    key: &str,
    source: Option<&str>,
) -> AvengerApp<ChartAppState> {
    let point_key = match source {
        Some(source) => PointKey::new(key).source(source),
        None => PointKey::new(key),
    };
    let points = PointSelect::builder().keys([point_key]).toggle(true);
    let bars = CartesianLayer::bar(counts.clone(), BarMark::builder().x(key).y("cars"));
    let chart = Chart::builder(
        Plot::cartesian([bars])
            .id("plot")
            .tools([Select::points("brush", points)]),
    )
    .selections([Selection::new("brush", SelectionResolution::Union)])
    .build()
    .unwrap();
    app(chart).await.0
}

/// The middle of the bar of `row` of `counts`, whose first column the bars' x encodes.
fn bar_of(app: &AvengerApp<ChartAppState>, counts: &RecordBatch, row: usize) -> [f32; 2] {
    let plot = &app.state().plots()[0];
    let column = counts.column(0);
    let x = avenger_chart::lump::keys(column).unwrap_or_else(|| column.clone());
    let cars = counts
        .column(1)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    let y: arrow::array::ArrayRef = Arc::new(Float64Array::from(vec![cars.value(row) / 2.0]));
    let [x, y] = plot.coords.pixels([&x.slice(row, 1), &y]).unwrap();
    [plot.rect[0] + x.value(0), plot.rect[1] + y.value(0)]
}

/// A click at `at`.
async fn click_at(app: &mut AvengerApp<ChartAppState>, at: [f32; 2]) {
    send(
        app,
        [
            moved(at),
            mouse(ElementState::Pressed),
            mouse(ElementState::Released),
        ],
    )
    .await;
}

/// The values of the one set of the selection `brush` over the source `cylinders`, NULL as
/// `None`, sorted, and whether it is a complement.
fn cylinders_set(app: &AvengerApp<ChartAppState>) -> (Vec<Option<i64>>, bool) {
    match &brush(app)[..] {
        [clause] => match &clause.conditions[..] {
            [avenger_selection::Condition::Set {
                columns,
                values,
                complement,
                ..
            }] => {
                assert_eq!(columns, &["cylinders".to_string()]);
                let values = values[0]
                    .as_any()
                    .downcast_ref::<arrow::array::Int64Array>()
                    .unwrap();
                let mut values: Vec<Option<i64>> = values.iter().collect();
                values.sort();
                (values, *complement)
            }
            conditions => panic!("a set, and not {conditions:?}"),
        },
        clauses => panic!("one clause, and not {clauses:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_of_the_null_bar_of_a_column_selects_its_null_rows() {
    let counts = RecordBatch::try_from_iter([
        (
            "origin",
            Arc::new(StringArray::from(vec![Some("USA"), None, Some("Japan")]))
                as arrow::array::ArrayRef,
        ),
        (
            "cars",
            Arc::new(Float64Array::from(vec![249.0, 14.0, 79.0])),
        ),
    ])
    .unwrap();
    let mut app = bars_app(&counts, "origin", None).await;
    let at = bar_of(&app, &counts, 1);
    click_at(&mut app, at).await;
    let clauses = brush(&app);
    let [clause] = &clauses[..] else {
        panic!("one clause, and not {clauses:?}");
    };
    // The set matches the cars of another table without an origin, and no other car.
    let origins: arrow::array::ArrayRef = Arc::new(StringArray::from(vec![
        Some("Japan"),
        None,
        Some("USA"),
        None,
    ]));
    let matched = clause.conditions[0].matches(&[origins]).unwrap();
    let rows: Vec<usize> = (0..matched.len())
        .filter(|&row| matched.value(row))
        .collect();
    assert_eq!(rows, [1, 3]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_of_the_null_bar_of_a_lump_selects_null_and_the_other_bar_leaves_it_out() {
    // 4 and 8 are kept, 6, 3, and 5 are the Other group, and the NULL category is its own.
    let counts = lumped_counts(
        vec![Some(4), Some(8), None, None],
        vec![None, None, Some("Other"), None],
        vec![207.0, 108.0, 91.0, 5.0],
        None,
    );
    let mut app = bars_app(&counts, "lumped", Some("cylinders")).await;
    let null = bar_of(&app, &counts, 3);
    click_at(&mut app, null).await;
    assert_eq!(cylinders_set(&app), (vec![None], false));

    // A Shift click of the Other bar adds it, so the complement leaves out the kept 4 and 8.
    std::thread::sleep(Duration::from_millis(600));
    let other = bar_of(&app, &counts, 2);
    send(&mut app, shift_click(other)).await;
    assert_eq!(cylinders_set(&app), (vec![Some(4), Some(8)], true));

    // A click of the Other bar alone leaves out the kept 4 and 8 and the NULL category.
    std::thread::sleep(Duration::from_millis(600));
    click_at(&mut app, other).await;
    assert_eq!(cylinders_set(&app), (vec![None, Some(4), Some(8)], true));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_of_the_other_bar_of_a_lump_that_drops_null_leaves_null_out() {
    // The last row is a NULL lump, the cars of an unknown number of cylinders that the lump drops.
    let counts = lumped_counts(
        vec![Some(4), Some(8), None, None],
        vec![None, None, Some("Other"), None],
        vec![207.0, 108.0, 91.0, 5.0],
        Some(vec![true, true, true, false]),
    );
    let mut app = bars_app(&counts, "lumped", Some("cylinders")).await;
    let other = bar_of(&app, &counts, 2);
    click_at(&mut app, other).await;
    assert_eq!(cylinders_set(&app), (vec![None, Some(4), Some(8)], true));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_writes_the_clause_of_its_plot_and_grays_the_rows_outside_it() {
    let select = vec![ToolBuilder::from(Select::interval(
        "brush",
        IntervalSelect::builder(),
    ))];
    let (mut app, _, changes) = app(scatter(cars(), select)).await;
    let [x, y, width, height] = app.state().plots()[0].rect;
    let from = [x + width * 0.25, y + height * 0.1];
    let to = [x + width * 0.6, y + height * 0.9];
    send(&mut app, drag(from, to)).await;

    let clauses = &app.state().evaluator().selections()["brush"].clauses;
    assert_eq!(clauses.len(), 1);
    assert_eq!(clauses[0].owner, "plot");
    assert_eq!(clauses[0].conditions.len(), 2);
    // The drag reports each move and its settle.
    let changes = changes.lock().unwrap();
    assert!(changes.len() >= 2, "{changes:?}");
    assert!(changes.last().unwrap().settled);
    assert_eq!(changes.last().unwrap().selection("brush").unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_that_reports_settled_changes_reports_a_drag_once() {
    let select = vec![ToolBuilder::from(Select::interval(
        "brush",
        IntervalSelect::builder(),
    ))];
    let (mut app, _, changes) = app_reporting(scatter(cars(), select), Reports::Settled).await;
    let [x, y, width, height] = app.state().plots()[0].rect;
    send(
        &mut app,
        drag(
            [x + width * 0.25, y + height * 0.1],
            [x + width * 0.6, y + height * 0.9],
        ),
    )
    .await;
    let changes = changes.lock().unwrap();
    let [change] = &changes[..] else {
        panic!("one settled change, and not {changes:?}");
    };
    assert!(change.settled);
    assert_eq!(change.selection("brush").unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_mode_decides_whether_a_drag_pans_or_draws_a_box() {
    let tools = vec![
        ToolBuilder::from(Navigate::builder()).modes(["pan"]),
        ToolBuilder::from(Select::interval("brush", IntervalSelect::builder())).modes(["box"]),
    ];
    let (mut app, updates, _) = app(scatter(cars(), tools)).await;
    let fitted = domains(&app, 0);
    let [x, y, width, height] = app.state().plots()[0].rect;
    let (from, to) = (
        [x + width * 0.25, y + height * 0.25],
        [x + width * 0.6, y + height * 0.75],
    );
    send(&mut app, drag(from, to)).await;
    assert_ne!(domains(&app, 0), fitted);
    assert!(app.state().evaluator().selections()["brush"]
        .clauses
        .is_empty());

    // The next press stands apart from the last release, so it is no double click.
    std::thread::sleep(Duration::from_millis(600));
    updates.set_mode("box", true);
    app.refresh().await;
    assert_eq!(app.state().modes(), ["box"]);
    let panned = domains(&app, 0);
    send(&mut app, drag(from, to)).await;
    assert_eq!(
        app.state().evaluator().selections()["brush"].clauses.len(),
        1
    );
    assert_eq!(domains(&app, 0), panned);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_active_box_takes_the_drag_and_the_navigation_the_wheel() {
    // Both tools are active in every mode.
    let tools = vec![
        ToolBuilder::from(Navigate::builder()),
        Select::interval("brush", IntervalSelect::builder()).into(),
    ];
    let (mut app, _, _) = app(scatter(cars(), tools)).await;
    let fitted = domains(&app, 0);
    let [x, y, width, height] = app.state().plots()[0].rect;
    let (from, to) = (
        [x + width * 0.25, y + height * 0.25],
        [x + width * 0.6, y + height * 0.75],
    );
    send(&mut app, drag(from, to)).await;
    assert_eq!(domains(&app, 0), fitted);
    assert_eq!(
        app.state().evaluator().selections()["brush"].clauses.len(),
        1
    );
    let at = center(&app, 0);
    send(&mut app, [moved(at), wheel(2.0)]).await;
    assert_ne!(domains(&app, 0), fitted);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_chart_keeps_the_views_of_the_scales_that_it_still_has() {
    let (mut app, updates, _) = app(facets(&["USA", "Japan"], weight_family, navigate)).await;
    let at = center(&app, 0);
    send(&mut app, [moved(at), wheel(2.0)]).await;
    let zoomed = domains(&app, 0);

    // The family keeps its view, and so does the shared `y`, whose first plot is still USA.
    updates.send(facets(&["USA", "Europe", "Japan"], weight_family, navigate));
    refresh_until(&mut app, |app| app.state().plots().len() == 3).await;
    for plot in 0..3 {
        assert_eq!(domains(&app, plot), zoomed);
    }

    // A chart with another family and other facets keeps neither view.
    let mass = || Scale::builder().family("mass");
    updates.send(facets(&["Europe", "Japan"], mass, navigate));
    refresh_until(&mut app, |app| app.state().plots().len() == 2).await;
    let evaluator = app.state().evaluator();
    assert!((0..2).all(|plot| [Channel::X, Channel::Y]
        .iter()
        .all(|channel| evaluator.scale_view(plot, *channel).is_none())));
    assert_ne!(domains(&app, 0)[0], zoomed[0]);
}

/// A scatter plot of `cars` without an id that navigates: the root, or the first of two plots in a
/// row, at another path.
fn unnamed(cars: RecordBatch, in_a_row: bool) -> Chart {
    let plot = || {
        let symbols =
            CartesianLayer::symbol(cars.clone(), SymbolMark::builder().x("weight").y("mpg"));
        Plot::cartesian([symbols]).tools(navigate())
    };
    match in_a_row {
        false => Chart::builder(plot()).build().unwrap(),
        true => Chart::builder(Concat::new(Direction::Row).children([plot(), plot()]))
            .build()
            .unwrap(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plot_without_an_id_keeps_its_view_while_the_chart_keeps_its_structure() {
    let (mut app, updates, _) = app(unnamed(cars(), false)).await;
    let at = center(&app, 0);
    send(&mut app, [moved(at), wheel(2.0)]).await;
    let zoomed = domains(&app, 0);
    assert!(app.state().evaluator().scale_view(0, Channel::X).is_some());

    // New rows in the same structure keep the view, by the plot's path.
    let fewer = cars_of(&["USA", "Japan"]);
    let rows = fewer.num_rows();
    updates.send(unnamed(fewer, false));
    refresh_until(&mut app, |app| {
        app.state().chart().tables()[0].num_rows() == rows
    })
    .await;
    assert_eq!(domains(&app, 0), zoomed);

    // The plot at another path starts at its fit.
    updates.send(unnamed(cars(), true));
    refresh_until(&mut app, |app| app.state().plots().len() == 2).await;
    let evaluator = app.state().evaluator();
    assert!((0..2).all(|plot| evaluator.scale_view(plot, Channel::X).is_none()));
}

#[tokio::test(flavor = "multi_thread")]
async fn updates_apply_in_the_order_of_their_calls_and_a_newer_chart_overtakes_an_older_one() {
    let (mut app, updates, _) = app(scatter(cars(), navigate())).await;
    // An older chart, a mode, and a newer chart that overtakes the older one.
    updates.send(scatter(cars_of(&["USA"]), navigate()));
    updates.set_mode("box", true);
    updates.send(scatter(cars_of(&["Japan"]), navigate()));
    let japan = cars_of(&["Japan"]).num_rows();
    refresh_until(&mut app, |app| {
        app.state().chart().tables()[0].num_rows() == japan
    })
    .await;
    assert_eq!(app.state().modes(), ["box"]);
    assert!(updates.error().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chart_that_fails_to_prepare_keeps_the_chart_and_reports_its_error() {
    let (mut app, updates, _) = app(scatter(cars(), navigate())).await;
    // A chart whose formats need a provider that the label engine lacks.
    let symbols = CartesianLayer::symbol(cars(), SymbolMark::builder().x("weight").y("mpg"));
    let unformatted = Chart::builder(Plot::cartesian([symbols]))
        .number_format_syntax("icu")
        .datetime_format_syntax("d3")
        .build()
        .unwrap();
    updates.send(unformatted);
    refresh_until(&mut app, |_| updates.error().is_some()).await;
    assert_eq!(
        app.state().chart().tables()[0].num_rows(),
        cars().num_rows()
    );
    // A later mode still applies.
    updates.set_mode("box", true);
    app.refresh().await;
    assert_eq!(app.state().modes(), ["box"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_that_keeps_its_scales_keeps_the_home_of_a_double_click() {
    let (mut app, updates, _) = app(scatter(cars(), navigate())).await;
    let fitted = domains(&app, 0);
    let usa = cars_of(&["USA"]);
    let rows = usa.num_rows();
    updates.send(ChartUpdate::new(scatter(usa, navigate())).fit(Fit::Keep));
    refresh_until(&mut app, |app| {
        app.state().chart().tables()[0].num_rows() == rows
    })
    .await;
    assert_eq!(domains(&app, 0), fitted);

    let at = center(&app, 0);
    send(&mut app, [moved(at), wheel(2.0)]).await;
    assert_ne!(domains(&app, 0), fitted);
    send(&mut app, double_click(at)).await;
    assert_eq!(domains(&app, 0), fitted);
}

#[tokio::test(flavor = "multi_thread")]
async fn new_style_inputs_restyle_the_chart_without_a_new_chart() {
    let (mut app, updates, _) = app(scatter(cars(), navigate())).await;
    let marks = |app: &AvengerApp<ChartAppState>| app.scenes()[0].marks.len();
    let before = marks(&app);
    // A stylesheet that hides the axes.
    updates.send(ChartUpdate::styles(cascade_with("axis { display: none; }")));
    app.refresh().await;
    assert!(marks(&app) < before, "{} {before}", marks(&app));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_press_without_a_drag_leaves_the_scales_fitting() {
    let (mut app, updates, _) = app(scatter(cars(), navigate())).await;
    let at = center(&app, 0);
    send(
        &mut app,
        [
            moved(at),
            mouse(ElementState::Pressed),
            mouse(ElementState::Released),
        ],
    )
    .await;
    let usa = cars_of(&["USA"]);
    let rows = usa.num_rows();
    updates.send(scatter(usa.clone(), navigate()));
    refresh_until(&mut app, |app| {
        app.state().chart().tables()[0].num_rows() == rows
    })
    .await;
    let (alone, _, _) = self::app(scatter(usa, navigate())).await;
    assert_eq!(domains(&app, 0), domains(&alone, 0));
    assert!(app.state().evaluator().scale_view(0, Channel::X).is_none());
}

/// A scatter plot `plot` of the cars, whose layer `points` declares `layer_tools`, and whose plot
/// declares `tools`.
fn scatter_of(tools: Vec<ToolBuilder>, layer_tools: Vec<LayerToolBuilder>) -> Chart {
    let symbols = CartesianLayer::symbol(cars(), SymbolMark::builder().x("weight").y("mpg"))
        .id("points")
        .tools(layer_tools);
    Chart::builder(Plot::cartesian([symbols]).id("plot").tools(tools))
        .selections([Selection::new("brush", SelectionResolution::Union)])
        .build()
        .unwrap()
}

/// The clauses of the selection `brush`.
fn brush(app: &AvengerApp<ChartAppState>) -> Vec<avenger_selection::Clause> {
    app.state().evaluator().selections()["brush"]
        .clauses
        .clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_of_a_layer_writes_the_clause_of_the_layer() {
    let select = Select::interval("brush", IntervalSelect::builder());
    let (mut app, _, _) = app(scatter_of(Vec::new(), vec![select.into()])).await;
    let [x, y, width, height] = app.state().plots()[0].rect;
    send(
        &mut app,
        drag(
            [x + width * 0.2, y + height * 0.2],
            [x + width * 0.7, y + height * 0.8],
        ),
    )
    .await;
    let clauses = brush(&app);
    assert_eq!(clauses.len(), 1);
    assert_eq!(clauses[0].owner, "points");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lasso_writes_a_polygon_of_the_recorded_columns() {
    let lasso = Select::lasso("brush", Vec::<String>::new());
    let (mut app, _, _) = app(scatter_of(vec![lasso.into()], Vec::new())).await;
    let [x, y, width, height] = app.state().plots()[0].rect;
    let path = [
        [x + width * 0.2, y + height * 0.2],
        [x + width * 0.8, y + height * 0.25],
        [x + width * 0.7, y + height * 0.8],
        [x + width * 0.25, y + height * 0.7],
    ];
    let mut events = vec![moved(path[0]), mouse(ElementState::Pressed)];
    events.extend(path[1..].iter().map(|at| moved(*at)));
    events.push(mouse(ElementState::Released));
    send(&mut app, events).await;
    let clauses = brush(&app);
    assert_eq!(clauses.len(), 1);
    match &clauses[0].conditions[..] {
        [avenger_selection::Condition::Polygon {
            x_column, y_column, ..
        }] => assert_eq!((x_column.as_str(), y_column.as_str()), ("weight", "mpg")),
        conditions => panic!("a polygon, and not {conditions:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_of_a_point_selection_writes_the_keys_of_the_instance() {
    let points = Select::points("brush", PointSelect::builder().keys(["origin"]));
    let (mut app, _, _) = app(scatter_of(vec![points.into()], Vec::new())).await;
    let cars = cars();
    let at = first_car(&app, 0, &cars);
    send(
        &mut app,
        [
            moved(at),
            mouse(ElementState::Pressed),
            mouse(ElementState::Released),
        ],
    )
    .await;
    let clauses = brush(&app);
    assert_eq!(clauses.len(), 1);
    match &clauses[0].conditions[..] {
        [avenger_selection::Condition::Set {
            columns, values, ..
        }] => {
            assert_eq!(columns, &["origin".to_string()]);
            let value = values[0].as_any().downcast_ref::<StringArray>().unwrap();
            assert_eq!(value.value(0), "USA");
        }
        conditions => panic!("a set, and not {conditions:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_of_an_instance_whose_key_is_null_selects_the_null_rows() {
    // The cars, where the first car and one other have no name.
    let cars = cars();
    let names: Vec<Option<String>> = (0..cars.num_rows())
        .map(|row| (row != 0 && row != 5).then(|| format!("car {row}")))
        .collect();
    let named = RecordBatch::try_from_iter([
        ("weight", cars.column_by_name("weight").unwrap().clone()),
        ("mpg", cars.column_by_name("mpg").unwrap().clone()),
        (
            "name",
            Arc::new(StringArray::from(names)) as arrow::array::ArrayRef,
        ),
    ])
    .unwrap();
    let symbols = CartesianLayer::symbol(named.clone(), SymbolMark::builder().x("weight").y("mpg"))
        .id("points");
    let points = Select::points("brush", PointSelect::builder().keys(["name"]));
    let chart = Chart::builder(Plot::cartesian([symbols]).id("plot").tools([points]))
        .selections([Selection::new("brush", SelectionResolution::Union)])
        .build()
        .unwrap();
    let (mut app, _, _) = app(chart).await;
    let at = first_car(&app, 0, &named);
    send(
        &mut app,
        [
            moved(at),
            mouse(ElementState::Pressed),
            mouse(ElementState::Released),
        ],
    )
    .await;
    let clauses = brush(&app);
    let [clause] = &clauses[..] else {
        panic!("one clause, and not {clauses:?}");
    };
    let [condition] = &clause.conditions[..] else {
        panic!("one condition, and not {clause:?}");
    };
    // The set holds a null, which matches both cars without a name and no other.
    let matched = condition
        .matches(&[named.column_by_name("name").unwrap().clone()])
        .unwrap();
    let rows: Vec<usize> = (0..matched.len())
        .filter(|&row| matched.value(row))
        .collect();
    assert_eq!(rows, [0, 5]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_in_a_polar_plot_writes_ranges_of_its_angle_and_its_radius() {
    let symbols = PolarLayer::symbol(
        cars(),
        PolarSymbolMark::builder().theta("weight").radius("mpg"),
    );
    let chart = Chart::builder(
        Plot::polar([symbols])
            .id("plot")
            .tools([Select::interval("brush", IntervalSelect::builder())]),
    )
    .selections([Selection::new("brush", SelectionResolution::Union)])
    .build()
    .unwrap();
    let (mut app, _, _) = app(chart).await;
    let [x, y, width, height] = app.state().plots()[0].rect;
    let middle = [x + width / 2.0, y + height / 2.0];
    send(
        &mut app,
        drag(
            [middle[0] + width * 0.05, middle[1] - height * 0.4],
            [middle[0] + width * 0.3, middle[1] - height * 0.1],
        ),
    )
    .await;
    let clauses = brush(&app);
    assert_eq!(clauses.len(), 1);
    let columns: Vec<String> = (clauses[0].conditions.iter())
        .flat_map(|condition| condition.columns().into_iter().map(str::to_string))
        .collect();
    assert_eq!(columns, ["weight", "mpg"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_of_cells_reports_its_focus_under_the_pointer() {
    let select = Select::interval("brush", IntervalSelect::builder().cell(10.0));
    let updates = ChartUpdates::new();
    let focus = Arc::new(Mutex::new(Vec::new()));
    let heard = focus.clone();
    let mut app = ChartApp::new(
        scatter_of(vec![select.into()], Vec::new()),
        cascade(),
        engine(),
    )
    .size(SIZE)
    .updates(&updates)
    .on_focus(move |focus| heard.lock().unwrap().push(focus.cloned()))
    .build()
    .await
    .unwrap()
    .with_runtime(tokio::runtime::Handle::current())
    .unwrap();
    let at = center(&app, 0);
    send(&mut app, [moved(at)]).await;
    let focus = focus.lock().unwrap();
    let Some(Some(focused)) = focus.last() else {
        panic!("a focus, and not {focus:?}");
    };
    assert_eq!(focused.selection, "brush");
    let columns: Vec<&str> = (focused.dimensions.iter())
        .map(|dimension| dimension.column.as_str())
        .collect();
    assert_eq!(columns, ["weight", "mpg"]);
    assert!(focused
        .dimensions
        .iter()
        .all(|dimension| dimension.grid.is_some()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_navigation_of_a_selection_domain_moves_the_selection() {
    let symbols =
        CartesianLayer::symbol(
            cars(),
            SymbolMark::builder()
                .x(Encoding::scaled_field("weight").scale(Domain::selection(
                    SelectionDomain::data("brush", Channel::X, DataDomain::builder()),
                )))
                .y("mpg"),
        );
    let chart = Chart::builder(
        Plot::cartesian([symbols])
            .id("plot")
            .tools([Navigate::builder().channels([Channel::X])]),
    )
    .selections([Selection::new("brush", SelectionResolution::Union)])
    .build()
    .unwrap();
    let (mut app, _, changes) = app(chart).await;
    let fitted = domains(&app, 0)[0];
    let at = center(&app, 0);
    send(&mut app, drag(at, [at[0] + 50.0, at[1]])).await;
    app.refresh().await;
    // The pan writes a range of `weight` to the selection, which the domain reads.
    let clauses = brush(&app);
    assert_eq!(clauses.len(), 1);
    assert_ne!(domains(&app, 0)[0], fitted);
    let changes = changes.lock().unwrap();
    assert!(changes
        .iter()
        .any(|change| change.selection("brush").is_some()));
}
