//! Selections of a chart app driven by window events: boxes, clicks, and lassos that write the
//! declared selections, the plots that draw them, their conditions, the selection domains that
//! link a brush to a navigation, the interaction mode, and selections from the feed.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch, StringArray},
    compute::{filter_record_batch, kernels::cmp::eq},
};
use avenger_app::app::AvengerApp;
use avenger_chart::{
    proto::{self, select, Channel, SelectionResolution},
    Chart, Contribution, Encode, Scale, Selection, TupleTest, UnitBuilder, Value,
};
use avenger_chart_app::{
    Carry, ChartApp, ChartAppState, ChartChange, ChartFeed, ChartUpdate, Mode,
};
use avenger_chart_eval::PlotCoord;
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, NamedKey, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
    WindowMouseInput,
};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_scenegraph::marks::mark::SceneMark;

const SIZE: [f32; 2] = [900.0, 400.0];

/// A table of 24 cars: a weight, a fuel economy, an acceleration, an origin, and a name.
fn cars() -> RecordBatch {
    let rows = 24;
    let weight: Vec<f64> = (0..rows)
        .map(|row| 1800.0 + ((row * 37) % 24) as f64 * 130.0)
        .collect();
    let mpg: Vec<f64> = weight
        .iter()
        .enumerate()
        .map(|(row, weight)| 48.0 - weight / 140.0 + ((row * 7) % 5) as f64)
        .collect();
    let acceleration: Vec<f64> = (0..rows)
        .map(|row| 10.0 + ((row * 11) % 13) as f64)
        .collect();
    let origins = ["USA", "Europe", "Japan"];
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("weight", Arc::new(Float64Array::from(weight))),
        ("mpg", Arc::new(Float64Array::from(mpg))),
        ("acceleration", Arc::new(Float64Array::from(acceleration))),
        (
            "origin",
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| origins[row % 3]),
            )),
        ),
        (
            "name",
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| format!("car {row}")),
            )),
        ),
    ];
    RecordBatch::try_from_iter(columns).unwrap()
}

fn cars_of(origin: &str) -> RecordBatch {
    let cars = cars();
    let keep = eq(
        cars.column_by_name("origin").unwrap(),
        &StringArray::new_scalar(origin),
    )
    .unwrap();
    filter_record_batch(&cars, &keep).unwrap()
}

/// A predicate of the rows that the selection `name` selects, which selects none while the
/// selection is empty.
fn selected(name: &str) -> proto::Predicate {
    proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selection(
            proto::SelectionTest {
                selection: name.into(),
                empty_selects_none: true,
                ..Default::default()
            },
        )),
    }
}

/// A scatter plot with the id `id` of `y` against weight, whose rows that the selection `brush`
/// selects are red.
fn scatter(table: RecordBatch, id: &str, y: &str) -> UnitBuilder {
    Chart::unit(table, proto::SymbolMark::default())
        .id(id)
        .x("weight")
        .y(y)
        .color(Encode::constant("gray").when(selected("brush"), Value::Constant("red".into())))
}

/// A box on `channels`, or on `x` and `y` without channels, that writes the selection `brush`
/// with `scope`.
fn boxes(channels: &[Channel], scope: &[&str]) -> proto::Select {
    proto::Select {
        selection: "brush".into(),
        form: Some(select::Form::Interval(proto::IntervalSelect {
            channels: channels.iter().map(|channel| *channel as i32).collect(),
        })),
        scope: scope.iter().map(|column| column.to_string()).collect(),
    }
}

fn brush(resolution: SelectionResolution) -> Selection {
    Selection::new("brush", resolution)
}

type Changes = Arc<Mutex<Vec<ChartChange>>>;

async fn start(chart: Chart) -> (AvengerApp<ChartAppState>, ChartFeed, Changes) {
    let feed = ChartFeed::new();
    let changes = Changes::default();
    let heard = changes.clone();
    let engine = avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()));
    let app = ChartApp::new(chart, engine)
        .size(SIZE)
        .feed(&feed)
        .on_change(move |change| heard.lock().unwrap().push(change.clone()))
        .build()
        .await
        .unwrap();
    (app, feed, changes)
}

/// Sends events 10 ms apart, so the throttles of drags take each move.
async fn send(app: &mut AvengerApp<ChartAppState>, events: impl IntoIterator<Item = WindowEvent>) {
    for event in events {
        std::thread::sleep(Duration::from_millis(10));
        app.update(&event, Instant::now()).await;
    }
}

fn moved(position: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position })
}

fn mouse(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

fn shift(state: ElementState) -> WindowEvent {
    WindowEvent::KeyboardInput(WindowKeyboardInput {
        key: Key::Named(NamedKey::Shift),
        state,
        text: None,
    })
}

fn press(at: [f32; 2]) -> [WindowEvent; 2] {
    [moved(at), mouse(ElementState::Pressed)]
}

fn drag(from: [f32; 2], to: [f32; 2]) -> [WindowEvent; 5] {
    let middle = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    [
        moved(from),
        mouse(ElementState::Pressed),
        moved(middle),
        moved(to),
        mouse(ElementState::Released),
    ]
}

fn click(at: [f32; 2]) -> [WindowEvent; 3] {
    [
        moved(at),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
    ]
}

fn double_click(at: [f32; 2]) -> [WindowEvent; 5] {
    [
        moved(at),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
    ]
}

/// The scales of `x` and `y` of the plot at `plot`.
fn scales(
    app: &AvengerApp<ChartAppState>,
    plot: usize,
) -> [avenger_scales::scales::ConfiguredScale; 2] {
    let PlotCoord::Cartesian {
        x: Some(x),
        y: Some(y),
        ..
    } = &app.state().plots()[plot].coord
    else {
        panic!("a Cartesian plot with x and y");
    };
    [x.clone(), y.clone()]
}

/// The position in the window of the values `[x, y]` of the plot at `plot`.
fn at(app: &AvengerApp<ChartAppState>, plot: usize, values: [f64; 2]) -> [f32; 2] {
    let rect = app.state().plots()[plot].rect;
    let [x, y] = scales(app, plot);
    let position = |scale: &avenger_scales::scales::ConfiguredScale, value: f64| {
        let value: ArrayRef = Arc::new(Float64Array::from(vec![value]));
        scale.scale_to_numeric(&value).unwrap().as_vec(1, None)[0]
    };
    [
        rect[0] + position(&x, values[0]),
        rect[1] + position(&y, values[1]),
    ]
}

/// The domain of the `x` scale of the plot at `plot`.
fn x_domain(app: &AvengerApp<ChartAppState>, plot: usize) -> (f64, f64) {
    scales(app, plot)[0].numeric_interval_domain().unwrap()
}

/// The boxes that the brush scene draws: the top-left corner of the area of each, and the left
/// of the box in the window.
fn drawn_boxes(app: &AvengerApp<ChartAppState>) -> Vec<[f32; 3]> {
    fn walk(marks: &[SceneMark], origin: [f32; 2], found: &mut Vec<[f32; 3]>) {
        for mark in marks {
            match mark {
                SceneMark::Group(group) => {
                    let origin = [origin[0] + group.origin[0], origin[1] + group.origin[1]];
                    walk(&group.marks, origin, found);
                }
                SceneMark::Path(path) if path.name == "brush_box" => {
                    let points = path.path.first().into_iter().flat_map(|path| path.iter());
                    let left = points
                        .map(|event| event.to().x)
                        .fold(f32::INFINITY, f32::min);
                    found.push([origin[0], origin[1], origin[0] + left]);
                }
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    walk(&app.scenes()[1].marks, [0.0, 0.0], &mut found);
    found
}

/// The indices of the plots whose areas hold the boxes that the brush scene draws.
fn plots_with_boxes(app: &AvengerApp<ChartAppState>) -> Vec<usize> {
    let mut plots: Vec<usize> = drawn_boxes(app)
        .into_iter()
        .filter_map(|[x, y, _]| {
            (app.state().plots().iter())
                .position(|plot| (plot.rect[0] - x).abs() < 0.5 && (plot.rect[1] - y).abs() < 0.5)
        })
        .collect();
    plots.sort();
    plots
}

/// Whether each symbol of the plot at `plot` is red.
fn reds(app: &AvengerApp<ChartAppState>, plot: usize) -> Vec<bool> {
    let name = &app.state().plots()[plot].name;
    let group = (app.scenes()[0].marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Group(group) if group.name == *name => Some(group),
            _ => None,
        })
        .expect("the group of the plot");
    let symbol = (group.marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Symbol(symbol) => Some(symbol),
            _ => None,
        })
        .expect("the symbols of the plot");
    (symbol.fill.as_vec(symbol.len as usize, None).into_iter())
        .map(|fill| {
            matches!(fill, avenger_color::ColorOrGradient::Color(color)
                if color[0] > 0.9 && color[1] < 0.1)
        })
        .collect()
}

/// The contributions of the selection `name`.
fn contributions(app: &AvengerApp<ChartAppState>, name: &str) -> Vec<Contribution> {
    (app.state().evaluator().selections().get(name))
        .cloned()
        .unwrap_or_default()
}

/// The ends of the range of the column `column` in the first tuple of `contribution`.
fn range(contribution: &Contribution, column: &str) -> (f64, f64) {
    contribution.tuples[0]
        .iter()
        .find_map(|test| match test {
            TupleTest::Range {
                column: own, ends, ..
            } if own == column => {
                let ends = ends.as_any().downcast_ref::<Float64Array>().unwrap();
                Some((ends.value(0), ends.value(1)))
            }
            _ => None,
        })
        .expect("a range of the column")
}

#[tokio::test(flavor = "current_thread")]
async fn a_box_writes_its_selection_and_reports_it_settled_at_its_release() {
    let chart = scatter(cars(), "plot", "mpg")
        .tool(boxes(&[], &[]))
        .selection(brush(SelectionResolution::Union))
        .build()
        .unwrap();
    let (mut app, _, changes) = start(chart).await;
    let (from, to) = (at(&app, 0, [2500.0, 30.0]), at(&app, 0, [3500.0, 20.0]));
    send(&mut app, drag(from, to)).await;

    let written = contributions(&app, "brush");
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    assert_eq!(contribution.owner, "plot");
    let (low, high) = range(contribution, "weight");
    assert!((low - 2500.0).abs() < 10.0 && (high - 3500.0).abs() < 10.0);
    let (low, high) = range(contribution, "mpg");
    assert!((low - 20.0).abs() < 0.2 && (high - 30.0).abs() < 0.2);

    let changes = changes.lock().unwrap();
    let settled: Vec<bool> = (changes.iter())
        .map(|change| {
            assert!(change.domains.is_empty() && change.selection("brush").is_some());
            change.settled
        })
        .collect();
    assert_eq!(settled.last(), Some(&true));
    assert!(settled[..settled.len() - 1].iter().all(|settled| !settled));
    assert_eq!(plots_with_boxes(&app), [0]);
}

/// A box on a polar plot of weight around the circle and fuel economy out from its center
/// selects an annular sector through the plot's projection: a range of the hours of weight across
/// the circle, which wraps, and a range of fuel economy.
#[tokio::test(flavor = "current_thread")]
async fn a_box_on_a_polar_plot_selects_an_annular_sector() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .id("polar")
        .coord(proto::Polar::default())
        .theta("weight")
        .radius("mpg")
        .color(Encode::constant("gray").when(selected("brush"), Value::Constant("red".into())))
        .tool(boxes(&[], &[]))
        .selection(brush(SelectionResolution::Union))
        .build()
        .unwrap();
    let (mut app, _, _) = start(chart).await;
    let plot = app.state().plots()[0].clone();
    let PlotCoord::Polar {
        theta: Some(theta),
        radius: Some(radius),
        center,
        outer_radius,
        ..
    } = &plot.coord
    else {
        panic!("a polar plot with theta and radius");
    };
    let at = |angle: f32, distance: f32| {
        let (sin, cos) = angle.to_radians().sin_cos();
        [
            plot.rect[0] + center[0] + distance * sin,
            plot.rect[1] + center[1] - distance * cos,
        ]
    };
    let [near, far] = [0.4, 0.8].map(|fraction| outer_radius * fraction);
    send(&mut app, drag(at(30.0, near), at(60.0, far))).await;

    let written = contributions(&app, "brush");
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    let (domain_start, domain_end) = theta.numeric_interval_domain().unwrap();
    let hours = (contribution.tuples[0].iter())
        .find_map(|test| match test {
            TupleTest::CyclicRange {
                column,
                start,
                end,
                period,
                ..
            } if column == "weight" => Some((*start, *end, *period)),
            _ => None,
        })
        .expect("a cyclic range of weight");
    let [start, end] = [30.0, 60.0].map(|angle| theta.invert_scalar(angle).unwrap());
    assert!(
        (hours.0 - start).abs() < 10.0 && (hours.1 - end).abs() < 10.0,
        "{hours:?}"
    );
    assert!((hours.2 - (domain_end - domain_start).abs()).abs() < 1e-9);
    let (low, high) = range(contribution, "mpg");
    let [inner, outer] =
        [near, far].map(|distance| radius.invert_scalar(f64::from(distance)).unwrap());
    assert!((low - inner.min(outer)).abs() < 0.2 && (high - inner.max(outer)).abs() < 0.2);
    assert_eq!(plots_with_boxes(&app), [0]);
}

#[tokio::test(flavor = "current_thread")]
async fn a_condition_styles_the_rows_inside_a_box_during_its_drag() {
    let chart = scatter(cars(), "plot", "mpg")
        .tool(boxes(&[], &[]))
        .selection(brush(SelectionResolution::Union))
        .build()
        .unwrap();
    let (mut app, _, _) = start(chart).await;
    assert!(reds(&app, 0).iter().all(|red| !red));
    let (from, to) = (at(&app, 0, [2500.0, 30.0]), at(&app, 0, [3500.0, 20.0]));
    send(&mut app, press(from)).await;
    send(&mut app, [moved([from[0] + 5.0, from[1] + 5.0]), moved(to)]).await;
    let during = reds(&app, 0);
    assert!(during.iter().any(|red| *red) && !during.iter().all(|red| *red));
    send(&mut app, [mouse(ElementState::Released)]).await;
    assert_eq!(reds(&app, 0), during);
}

#[tokio::test(flavor = "current_thread")]
async fn a_box_shows_on_the_plots_that_draw_its_columns_and_styles_every_plot() {
    // Both plots draw weight, and only the first draws mpg.
    let chart = Chart::row([
        scatter(cars(), "economy", "mpg")
            .tool(boxes(&[], &[]))
            .selection(brush(SelectionResolution::Global))
            .build()
            .unwrap(),
        scatter(cars(), "speed", "acceleration")
            .tool(boxes(&[], &[]))
            .selection(brush(SelectionResolution::Global))
            .build()
            .unwrap(),
    ])
    .build()
    .unwrap();
    let (mut app, _, _) = start(chart).await;
    let (from, to) = (at(&app, 0, [2500.0, 30.0]), at(&app, 0, [3500.0, 20.0]));
    send(&mut app, drag(from, to)).await;
    assert_eq!(plots_with_boxes(&app), [0]);
    assert_eq!(reds(&app, 0), reds(&app, 1));
    assert!(reds(&app, 1).iter().any(|red| *red));

    // A box on `x` alone shows on both plots.
    let chart = Chart::row([
        scatter(cars(), "economy", "mpg")
            .tool(boxes(&[Channel::X], &[]))
            .selection(brush(SelectionResolution::Global))
            .build()
            .unwrap(),
        scatter(cars(), "speed", "acceleration")
            .tool(boxes(&[Channel::X], &[]))
            .selection(brush(SelectionResolution::Global))
            .build()
            .unwrap(),
    ])
    .build()
    .unwrap();
    let (mut app, _, _) = start(chart).await;
    let (from, to) = (at(&app, 0, [2500.0, 30.0]), at(&app, 0, [3500.0, 20.0]));
    send(&mut app, drag(from, to)).await;
    assert_eq!(plots_with_boxes(&app), [0, 1]);
}

/// A facet row of the cars of each origin, whose plots share the id `panel` and draw a box with
/// `scope` on `x` and `y`.
fn facets(resolution: SelectionResolution, scope: &[&str]) -> Chart {
    let panels: Vec<Chart> = ["USA", "Europe", "Japan"]
        .iter()
        .map(|origin| {
            scatter(cars_of(origin), "panel", "mpg")
                .partition("origin", *origin)
                .tool(boxes(&[], scope))
                .selection(brush(resolution))
                .build()
                .unwrap()
        })
        .collect();
    Chart::row(panels).facet("origin").build().unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn facets_show_boxes_by_the_resolution_and_the_scope_of_the_selection() {
    // A box over the whole plot, which holds every row of the plot.
    let wide = |app: &AvengerApp<ChartAppState>, plot: usize| {
        let [x, y, width, height] = app.state().plots()[plot].rect;
        ([x + 2.0, y + 2.0], [x + width - 2.0, y + height - 2.0])
    };

    // A global box shows in every facet.
    let (mut app, _, _) = start(facets(SelectionResolution::Global, &[])).await;
    let (from, to) = wide(&app, 0);
    send(&mut app, drag(from, to)).await;
    assert_eq!(plots_with_boxes(&app), [0, 1, 2]);
    assert!((0..3).all(|plot| reds(&app, plot).iter().all(|red| *red)));

    // A scope keeps the box, and the rows that it selects, in its facet.
    let (mut app, _, _) = start(facets(SelectionResolution::Global, &["origin"])).await;
    let (from, to) = wide(&app, 0);
    send(&mut app, drag(from, to)).await;
    assert_eq!(plots_with_boxes(&app), [0]);
    assert!(reds(&app, 0).iter().all(|red| *red));
    assert!(reds(&app, 1).iter().all(|red| !red));

    // A union keeps a box in each facet.
    let (mut app, _, _) = start(facets(SelectionResolution::Union, &[])).await;
    let (from, to) = wide(&app, 0);
    send(&mut app, drag(from, to)).await;
    let (from, to) = wide(&app, 2);
    send(&mut app, drag(from, to)).await;
    assert_eq!(plots_with_boxes(&app), [0, 2]);
    assert_eq!(contributions(&app, "brush").len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn an_overview_box_sets_the_domain_of_a_detail_that_pans_the_box() {
    let overview = scatter(cars(), "overview", "mpg")
        .tool(boxes(&[Channel::X], &[]))
        .selection(brush(SelectionResolution::Global))
        .build()
        .unwrap();
    let detail = Chart::unit(cars(), proto::SymbolMark::default())
        .id("detail")
        .x(Encode::column("weight").scale(Scale::new().selection("brush", Channel::X)))
        .y("mpg")
        .tool(proto::Navigate {
            channels: vec![Channel::X as i32],
        })
        .selection(brush(SelectionResolution::Global))
        .build()
        .unwrap();
    let chart = Chart::column([overview, detail]).build().unwrap();
    let (mut app, _, _) = start(chart).await;
    let fitted = x_domain(&app, 1);
    let (from, to) = (at(&app, 0, [2500.0, 30.0]), at(&app, 0, [3500.0, 20.0]));
    send(&mut app, drag(from, to)).await;
    let (low, high) = x_domain(&app, 1);
    assert!((low - 2500.0).abs() < 10.0 && (high - 3500.0).abs() < 10.0);
    let boxed = drawn_boxes(&app);

    // A pan of the detail moves the selection, which moves the box of the overview.
    let middle = at(&app, 1, [3000.0, 25.0]);
    send(&mut app, drag(middle, [middle[0] - 100.0, middle[1]])).await;
    let (panned_low, panned_high) = x_domain(&app, 1);
    assert!(panned_low > low + 50.0, "{panned_low} {low}");
    assert!((panned_high - panned_low - (high - low)).abs() < 1e-6);
    let written = contributions(&app, "brush");
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].owner, "detail");
    assert_eq!(range(&written[0], "weight"), (panned_low, panned_high));
    assert_eq!(plots_with_boxes(&app), [0]);
    assert_ne!(drawn_boxes(&app), boxed);

    // A double click on the detail clears the selection.
    send(&mut app, double_click(middle)).await;
    assert!(contributions(&app, "brush").is_empty());
    assert_eq!(x_domain(&app, 1), fitted);
    assert!(drawn_boxes(&app).is_empty());
}

/// The names of the cars that the selection `name` holds.
fn names(app: &AvengerApp<ChartAppState>, name: &str) -> Vec<String> {
    (contributions(app, name).iter())
        .flat_map(|contribution| contribution.tuples.iter().flatten())
        .flat_map(|test| match test {
            TupleTest::Set { column, values, .. } if column == "name" => {
                let values = values.as_any().downcast_ref::<StringArray>().unwrap();
                values.iter().flatten().map(str::to_string).collect()
            }
            _ => Vec::new(),
        })
        .collect()
}

/// A point selection that selects cars by name with clicks.
fn picks() -> proto::Select {
    proto::Select {
        selection: "brush".into(),
        form: Some(select::Form::Points(proto::PointSelect {
            key: "name".into(),
            toggle: true,
        })),
        scope: Vec::new(),
    }
}

/// The weight and the fuel economy of the car at `row`.
fn car(row: usize) -> [f64; 2] {
    let cars = cars();
    let value = |column: &str| {
        let values = cars.column_by_name(column).unwrap();
        values
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap()
            .value(row)
    };
    [value("weight"), value("mpg")]
}

#[tokio::test(flavor = "current_thread")]
async fn a_click_selects_the_key_of_its_row() {
    let chart = scatter(cars(), "plot", "mpg")
        .tool(picks())
        .selection(brush(SelectionResolution::Union))
        .build()
        .unwrap();
    let (mut app, _, _) = start(chart).await;
    let names = |app: &AvengerApp<ChartAppState>| names(app, "brush");
    let first = at(&app, 0, car(0));
    send(&mut app, click(first)).await;
    assert_eq!(names(&app), ["car 0"]);
    assert_eq!(reds(&app, 0).iter().filter(|red| **red).count(), 1);

    // A click with Shift adds a car.
    let second = at(&app, 0, car(1));
    send(&mut app, [shift(ElementState::Pressed)]).await;
    send(&mut app, click(second)).await;
    send(&mut app, [shift(ElementState::Released)]).await;
    assert_eq!(names(&app), ["car 0", "car 1"]);

    // A click away from every car clears the selection.
    let rect = app.state().plots()[0].rect;
    std::thread::sleep(Duration::from_millis(400));
    send(&mut app, click([rect[0] + 2.0, rect[1] + rect[3] - 2.0])).await;
    assert!(names(&app).is_empty());
}

/// A click acts on the point selections of its plot alone, so a click that clears a box in
/// another plot keeps the cars that clicks selected.
#[tokio::test(flavor = "current_thread")]
async fn a_click_in_another_plot_keeps_a_point_selection() {
    let chart = Chart::row([
        scatter(cars(), "picks", "mpg")
            .tool(picks())
            .selection(brush(SelectionResolution::Union))
            .build()
            .unwrap(),
        scatter(cars(), "boxes", "acceleration")
            .tool(proto::Select {
                selection: "box".into(),
                ..boxes(&[], &[])
            })
            .selection(brush(SelectionResolution::Union))
            .selection(Selection::new("box", SelectionResolution::Union))
            .build()
            .unwrap(),
    ])
    .build()
    .unwrap();
    let (mut app, _, _) = start(chart).await;
    let first = at(&app, 0, car(0));
    send(&mut app, click(first)).await;
    assert_eq!(names(&app, "brush"), ["car 0"]);
    let rect = app.state().plots()[1].rect;
    let inside = |fraction: [f32; 2]| {
        [
            rect[0] + rect[2] * fraction[0],
            rect[1] + rect[3] * fraction[1],
        ]
    };
    send(&mut app, drag(inside([0.2, 0.2]), inside([0.6, 0.6]))).await;
    assert_eq!(contributions(&app, "box").len(), 1);
    std::thread::sleep(Duration::from_millis(400));
    send(&mut app, click(inside([0.9, 0.9]))).await;
    assert!(contributions(&app, "box").is_empty());
    assert_eq!(names(&app, "brush"), ["car 0"]);
}

#[tokio::test(flavor = "current_thread")]
async fn a_drag_pans_in_navigate_mode_and_draws_a_box_in_select_mode() {
    let chart = scatter(cars(), "plot", "mpg")
        .tool(proto::Navigate::default())
        .tool(boxes(&[], &[]))
        .selection(brush(SelectionResolution::Union))
        .build()
        .unwrap();
    let (mut app, feed, _) = start(chart).await;
    assert_eq!(app.state().mode(), Mode::Navigate);
    let fitted = x_domain(&app, 0);
    let (from, to) = (at(&app, 0, [2500.0, 30.0]), at(&app, 0, [3500.0, 20.0]));
    send(&mut app, drag(from, to)).await;
    assert_ne!(x_domain(&app, 0), fitted);
    assert!(contributions(&app, "brush").is_empty());

    feed.set_mode(Mode::Select);
    app.refresh().await;
    assert_eq!(app.state().mode(), Mode::Select);
    let panned = x_domain(&app, 0);
    let (from, to) = (at(&app, 0, [3000.0, 30.0]), at(&app, 0, [3400.0, 20.0]));
    send(&mut app, drag(from, to)).await;
    assert_eq!(x_domain(&app, 0), panned);
    assert_eq!(contributions(&app, "brush").len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn a_selection_from_the_feed_draws_its_box_and_stays_across_a_swap() {
    let chart = |table: RecordBatch| {
        scatter(table, "plot", "mpg")
            .tool(boxes(&[], &[]))
            .selection(brush(SelectionResolution::Union))
            .build()
            .unwrap()
    };
    let (mut app, feed, changes) = start(chart(cars())).await;
    let ends = |low: f64, high: f64| -> ArrayRef { Arc::new(Float64Array::from(vec![low, high])) };
    feed.set_selection(
        "brush",
        vec![Contribution::new("plot").tuple([
            TupleTest::Range {
                column: "weight".into(),
                channel: Channel::X,
                ends: ends(2500.0, 3500.0),
                exclude_end: false,
            },
            TupleTest::Range {
                column: "mpg".into(),
                channel: Channel::Y,
                ends: ends(20.0, 30.0),
                exclude_end: false,
            },
        ])],
    );
    app.refresh().await;
    assert_eq!(plots_with_boxes(&app), [0]);
    let styled = reds(&app, 0);
    assert!(styled.iter().any(|red| *red));
    assert!(changes.lock().unwrap().is_empty());

    // A new chart keeps the selection by its name.
    feed.send(chart(cars_of("USA"))).unwrap();
    app.refresh().await;
    assert_eq!(contributions(&app, "brush").len(), 1);
    assert_eq!(plots_with_boxes(&app), [0]);

    feed.clear_selection("brush");
    app.refresh().await;
    assert!(drawn_boxes(&app).is_empty());
    assert!(reds(&app, 0).iter().all(|red| !red));
}

#[tokio::test(flavor = "current_thread")]
async fn a_lasso_writes_a_polygon_of_the_columns_of_the_plot() {
    let chart = scatter(cars(), "plot", "mpg")
        .tool(proto::Select {
            selection: "brush".into(),
            form: Some(select::Form::Lasso(proto::LassoSelect::default())),
            scope: Vec::new(),
        })
        .selection(brush(SelectionResolution::Union))
        .build()
        .unwrap();
    let (mut app, _, _) = start(chart).await;
    let corners = [
        [2200.0, 35.0],
        [4000.0, 35.0],
        [4000.0, 16.0],
        [2200.0, 16.0],
    ]
    .map(|corner| at(&app, 0, corner));
    send(&mut app, around(corners)).await;
    let written = contributions(&app, "brush");
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    assert!(matches!(
        &contribution.tuples[0][0],
        TupleTest::Polygon { x_column, y_column, .. } if x_column == "weight" && y_column == "mpg"
    ));
    let selected = reds(&app, 0).into_iter().filter(|red| *red).count();
    assert!(selected > 0 && selected < 24, "{selected}");
}

/// A lasso across the top of a polar plot writes a polygon on each side of the start of weight
/// around the circle, so its condition styles the rows inside the lasso on both sides.
#[tokio::test(flavor = "current_thread")]
async fn a_lasso_across_the_top_of_a_polar_plot_selects_both_sides() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .id("polar")
        .coord(proto::Polar::default())
        // The weights of the cars run around the whole circle, a step of 130 apart.
        .theta(
            Encode::column("weight")
                .scale(Scale::new().domain(Arc::new(Float64Array::from(vec![1800.0, 4920.0])))),
        )
        .radius("mpg")
        .color(Encode::constant("gray").when(selected("brush"), Value::Constant("red".into())))
        .tool(proto::Select {
            selection: "brush".into(),
            form: Some(select::Form::Lasso(proto::LassoSelect::default())),
            scope: Vec::new(),
        })
        .selection(brush(SelectionResolution::Union))
        .build()
        .unwrap();
    let (mut app, _, _) = start(chart).await;
    let plot = app.state().plots()[0].clone();
    let PlotCoord::Polar {
        center,
        outer_radius,
        ..
    } = &plot.coord
    else {
        panic!("a polar plot");
    };
    let at = |angle: f32, fraction: f32| {
        let (sin, cos) = angle.to_radians().sin_cos();
        let distance = outer_radius * fraction;
        [
            plot.rect[0] + center[0] + distance * sin,
            plot.rect[1] + center[1] - distance * cos,
        ]
    };
    // Out along a spoke between rows, around the rim, and back in along another.
    let corners = [
        at(292.5, 0.1),
        at(292.5, 0.98),
        at(330.0, 0.98),
        at(0.0, 0.98),
        at(30.0, 0.98),
        at(67.5, 0.98),
        at(67.5, 0.1),
    ];
    send(&mut app, around(corners)).await;

    let written = contributions(&app, "brush");
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    assert_eq!(
        contribution.tuples.len(),
        2,
        "a polygon on each side of the start"
    );
    // Each row is red when the lasso's path holds its symbol.
    let symbols = (app.scenes()[0].marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Group(group) if group.name == plot.name => {
                group.marks.iter().find_map(|mark| match mark {
                    SceneMark::Symbol(symbol) => Some(symbol.clone()),
                    _ => None,
                })
            }
            _ => None,
        })
        .expect("the symbols of the plot");
    let [xs, ys] = [&symbols.x, &symbols.y].map(|values| values.as_vec(symbols.len as usize, None));
    let holds = |[x, y]: [f32; 2]| {
        let mut inside = false;
        for (index, &[xi, yi]) in corners.iter().enumerate() {
            let [xj, yj] = corners[(index + corners.len() - 1) % corners.len()];
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                inside = !inside;
            }
        }
        inside
    };
    let reds = reds(&app, 0);
    let (mut left, mut right) = (0, 0);
    for (row, red) in reds.iter().enumerate() {
        let symbol = [plot.rect[0] + xs[row], plot.rect[1] + ys[row]];
        assert_eq!(*red, holds(symbol), "row {row} at {symbol:?}");
        if *red {
            if symbol[0] < plot.rect[0] + center[0] {
                left += 1;
            } else {
                right += 1;
            }
        }
    }
    assert!(
        left > 0 && right > 0,
        "{left} on the left and {right} on the right"
    );
}

/// A drag around `corners`, 10 pixels at a time, back to the first corner.
fn around<const N: usize>(corners: [[f32; 2]; N]) -> Vec<WindowEvent> {
    let mut events = vec![moved(corners[0]), mouse(ElementState::Pressed)];
    for (from, to) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        let steps = ((to[0] - from[0]).hypot(to[1] - from[1]) / 10.0).ceil() as usize;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            events.push(moved([
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ]));
        }
    }
    events.push(mouse(ElementState::Released));
    events
}

/// The number of lasso paths that the brush scene draws.
fn lasso_paths(app: &AvengerApp<ChartAppState>) -> usize {
    fn count(marks: &[SceneMark]) -> usize {
        (marks.iter())
            .map(|mark| match mark {
                SceneMark::Group(group) => count(&group.marks),
                SceneMark::Path(path) if path.name == "brush_lasso" => 1,
                _ => 0,
            })
            .sum()
    }
    count(&app.scenes()[1].marks)
}

#[tokio::test(flavor = "current_thread")]
async fn a_lasso_with_a_key_selects_the_keys_of_the_rows_inside_it() {
    let chart = scatter(cars(), "plot", "mpg")
        .tool(proto::Select {
            selection: "brush".into(),
            form: Some(select::Form::Lasso(proto::LassoSelect {
                key: "origin".into(),
            })),
            scope: Vec::new(),
        })
        .selection(brush(SelectionResolution::Union))
        .build()
        .unwrap();
    let (mut app, _, _) = start(chart).await;
    // A box of heavy cars of low economy, from more than one origin.
    let corners = [
        [3600.0, 25.0],
        [5000.0, 25.0],
        [5000.0, 10.0],
        [3600.0, 10.0],
    ]
    .map(|corner| at(&app, 0, corner));
    send(&mut app, around(corners)).await;
    let written = contributions(&app, "brush");
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    let [TupleTest::Set { column, values, .. }] = &contribution.tuples[0][..] else {
        panic!("one set of keys, and not {:?}", contribution.tuples);
    };
    assert_eq!(column, "origin");
    let held: Vec<&str> = (values
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .iter())
    .flatten()
    .collect();
    assert!(!held.is_empty());
    // Every car of an origin inside the lasso is selected, inside the path or not.
    let cars = cars();
    let origins = cars
        .column_by_name("origin")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    let expected: Vec<bool> = origins
        .iter()
        .map(|origin| held.contains(&origin.unwrap()))
        .collect();
    assert_eq!(reds(&app, 0), expected);
    // The path stays drawn after the release, while the selection holds the keys that it wrote.
    assert_eq!(lasso_paths(&app), 1);
    let away = at(&app, 0, [2500.0, 30.0]);
    send(&mut app, click(away)).await;
    assert!(contributions(&app, "brush").is_empty());
    assert_eq!(lasso_paths(&app), 0);
}

/// An overview whose box of `weight` and `mpg` writes the global selection `brush`, over a detail
/// whose `x` and `y` read the selection and whose navigation moves `channels`.
fn linked_detail(channels: &[Channel]) -> Chart {
    let overview = scatter(cars(), "overview", "mpg")
        .tool(boxes(&[], &[]))
        .selection(brush(SelectionResolution::Global))
        .build()
        .unwrap();
    let detail = Chart::unit(cars(), proto::SymbolMark::default())
        .id("detail")
        .x(Encode::column("weight").scale(Scale::new().selection("brush", Channel::X)))
        .y(Encode::column("mpg").scale(Scale::new().selection("brush", Channel::Y)))
        .tool(proto::Navigate {
            channels: channels.iter().map(|channel| *channel as i32).collect(),
        })
        .selection(brush(SelectionResolution::Global))
        .build()
        .unwrap();
    Chart::column([overview, detail]).build().unwrap()
}

/// A pan of a detail whose `x` and `y` read one selection writes one contribution with both
/// ranges, and reports the selection once when it settles.
#[tokio::test(flavor = "current_thread")]
async fn a_navigation_of_x_and_y_writes_both_ranges_of_one_selection() {
    let (mut app, _, changes) = start(linked_detail(&[Channel::X, Channel::Y])).await;
    let (from, to) = (at(&app, 0, [2500.0, 30.0]), at(&app, 0, [3500.0, 20.0]));
    send(&mut app, drag(from, to)).await;
    changes.lock().unwrap().clear();

    let middle = at(&app, 1, [3000.0, 25.0]);
    send(&mut app, drag(middle, [middle[0] - 60.0, middle[1] + 40.0])).await;
    let written = contributions(&app, "brush");
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    assert_eq!(contribution.owner, "detail");
    let [x, y] = scales(&app, 1).map(|scale| scale.numeric_interval_domain().unwrap());
    assert_eq!(range(contribution, "weight"), x);
    assert_eq!(range(contribution, "mpg"), y);
    assert!(
        x.0 > 2550.0 && y.0 > 20.5,
        "the pan moves both ranges: {x:?} {y:?}"
    );

    let changes = changes.lock().unwrap();
    let settled: Vec<&ChartChange> = changes.iter().filter(|change| change.settled).collect();
    let [change] = &settled[..] else {
        panic!("one settled change, and not {settled:?}");
    };
    assert!(change.domains.is_empty());
    assert_eq!(change.selections.len(), 1);
    assert_eq!(change.selection("brush").map(<[_]>::len), Some(1));
}

/// A pan of `x` alone keeps the range of `y` that a box wrote to the selection.
#[tokio::test(flavor = "current_thread")]
async fn a_navigation_of_x_keeps_the_range_of_y_of_a_box() {
    let (mut app, _, _) = start(linked_detail(&[Channel::X])).await;
    let (from, to) = (at(&app, 0, [2500.0, 30.0]), at(&app, 0, [3500.0, 20.0]));
    send(&mut app, drag(from, to)).await;
    let boxed = range(&contributions(&app, "brush")[0], "mpg");

    let middle = at(&app, 1, [3000.0, 25.0]);
    send(&mut app, drag(middle, [middle[0] - 60.0, middle[1]])).await;
    let written = contributions(&app, "brush");
    assert_eq!(written[0].owner, "detail");
    assert_eq!(range(&written[0], "mpg"), boxed);
    assert_eq!(scales(&app, 1)[1].numeric_interval_domain().unwrap(), boxed);
}

/// A scatter plot of `cars` whose box writes the global selection `brush`.
fn boxed(cars: RecordBatch) -> Chart {
    scatter(cars, "plot", "mpg")
        .tool(boxes(&[], &[]))
        .selection(brush(SelectionResolution::Global))
        .build()
        .unwrap()
}

/// An update with `Carry::Reset` clears a box at the build that installs its chart.
#[tokio::test(flavor = "current_thread")]
async fn an_update_that_resets_a_selection_clears_its_box_at_its_install() {
    let (mut app, feed, _) = start(boxed(cars())).await;
    let (from, to) = (at(&app, 0, [2500.0, 30.0]), at(&app, 0, [3500.0, 20.0]));
    send(&mut app, drag(from, to)).await;
    assert_eq!(contributions(&app, "brush").len(), 1);

    feed.send(ChartUpdate::new(boxed(cars_of("USA"))).selections(Carry::Reset))
        .unwrap();
    assert_eq!(contributions(&app, "brush").len(), 1);
    app.refresh().await;
    assert_eq!(
        app.state().chart().tables()[0].num_rows(),
        cars_of("USA").num_rows()
    );
    assert!(contributions(&app, "brush").is_empty());
    assert!(drawn_boxes(&app).is_empty());
}

/// A drag of a box continues across the install of a new chart, and writes its selection at the
/// release.
#[tokio::test(flavor = "current_thread")]
async fn a_box_continues_across_an_install() {
    let (mut app, feed, _) = start(boxed(cars())).await;
    let from = at(&app, 0, [2500.0, 30.0]);
    let to = at(&app, 0, [3500.0, 20.0]);
    let middle = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    send(&mut app, press(from)).await;
    send(&mut app, [moved(middle)]).await;

    feed.send(boxed(cars())).unwrap();
    app.refresh().await;
    send(&mut app, [moved(to), mouse(ElementState::Released)]).await;
    let written = contributions(&app, "brush");
    let [contribution] = &written[..] else {
        panic!("one contribution, and not {written:?}");
    };
    let (low, high) = range(contribution, "weight");
    assert!((low - 2500.0).abs() < 10.0 && (high - 3500.0).abs() < 10.0);
}
