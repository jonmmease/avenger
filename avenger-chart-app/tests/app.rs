//! A chart app driven by window events: navigation of the declared plots, tooltips, the changes
//! that the app reports, and new charts from the feed.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch, StringArray},
    compute::{filter_record_batch, kernels::cmp::eq},
};
use avenger_app::{app::AvengerApp, task::Task};
use avenger_chart::{
    proto::{self, scale_navigation, Channel},
    Chart, Encode, Scale,
};
use avenger_chart_app::{ChartApp, ChartAppState, ChartChange, ChartFeed};
use avenger_chart_eval::{PlotCoord, ScaleIdentity};
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, MouseScrollDelta, WindowCursorMoved, WindowEvent, WindowMouseInput,
    WindowMouseWheel,
};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_typst_label::LabelEngine;

const SIZE: [f32; 2] = [800.0, 400.0];

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// A table of 24 cars: a weight, a fuel economy, and an origin.
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
    let origins = ["USA", "Europe", "Japan"];
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("weight", Arc::new(Float64Array::from(weight))),
        ("mpg", Arc::new(Float64Array::from(mpg))),
        (
            "origin",
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| origins[row % 3]),
            )),
        ),
    ];
    RecordBatch::try_from_iter(columns).unwrap()
}

/// The cars of `origin`.
fn cars_of(origin: &str) -> RecordBatch {
    let cars = cars();
    let keep = eq(
        cars.column_by_name("origin").unwrap(),
        &StringArray::new_scalar(origin),
    )
    .unwrap();
    filter_record_batch(&cars, &keep).unwrap()
}

/// A facet row of the cars of each origin, whose plots share the id `panel` and an `x` in the
/// share family `weight`, and declare the tools of `tools` for their origin.
fn facets(origins: &[&str], family: &str, tools: impl Fn(&str) -> Vec<proto::Tool>) -> Chart {
    facets_of(origins, || Scale::new().family(family), tools)
}

/// A facet row of the cars of each origin, whose plots share the id `panel` and take the scale
/// that `x` gives for `x`, and declare the tools of `tools` for their origin.
fn facets_of(
    origins: &[&str],
    x: impl Fn() -> Scale,
    tools: impl Fn(&str) -> Vec<proto::Tool>,
) -> Chart {
    let panels: Vec<Chart> = origins
        .iter()
        .map(|origin| {
            let mut panel = Chart::unit(cars_of(origin), proto::SymbolMark::default())
                .id("panel")
                .partition("origin", *origin)
                .x(Encode::column("weight").scale(x()))
                .y("mpg");
            for tool in tools(origin) {
                panel = panel.tool(tool);
            }
            panel.build().unwrap()
        })
        .collect();
    Chart::row(panels)
        .facet("origin")
        .independent(Channel::X)
        .build()
        .unwrap()
}

/// A navigation of `x`.
fn navigate_x() -> proto::Tool {
    proto::Navigate {
        channels: vec![Channel::X as i32],
    }
    .into()
}

/// A chart app of `chart`, with a feed, that records the changes it reports.
async fn app(
    chart: Chart,
) -> (
    AvengerApp<ChartAppState>,
    ChartFeed,
    Arc<Mutex<Vec<ChartChange>>>,
) {
    let feed = ChartFeed::new();
    let changes = Arc::new(Mutex::new(Vec::new()));
    let heard = changes.clone();
    let app = ChartApp::new(chart, engine())
        .size(SIZE)
        .feed(&feed)
        .on_change(move |change| heard.lock().unwrap().push(change.clone()))
        .build()
        .await
        .unwrap();
    (app, feed, changes)
}

/// Sends events 10 ms apart, so the 8 ms throttle of a pan takes each move.
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

fn drag(from: [f32; 2], to: [f32; 2]) -> [WindowEvent; 4] {
    [
        moved(from),
        mouse(ElementState::Pressed),
        moved(to),
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

fn wheel(lines: f32) -> WindowEvent {
    WindowEvent::MouseWheel(WindowMouseWheel {
        delta: MouseScrollDelta::LineDelta(0.0, lines),
    })
}

/// The center of the plot at `plot`.
fn center(app: &AvengerApp<ChartAppState>, plot: usize) -> [f32; 2] {
    let [x, y, width, height] = app.state().plots()[plot].rect;
    [x + width / 2.0, y + height / 2.0]
}

/// The domains of the `x` and `y` scales of the plot at `plot`.
fn domains(app: &AvengerApp<ChartAppState>, plot: usize) -> [(f64, f64); 2] {
    let PlotCoord::Cartesian {
        x: Some(x),
        y: Some(y),
        ..
    } = &app.state().plots()[plot].coord
    else {
        panic!("a Cartesian plot with x and y");
    };
    [x, y].map(|scale| scale.numeric_interval_domain().unwrap())
}

#[tokio::test(flavor = "current_thread")]
async fn a_drag_in_one_facet_pans_every_facet_of_its_family() {
    let chart = facets(&["USA", "Japan"], "weight", |_| vec![navigate_x()]);
    let (mut app, _, _) = app(chart).await;
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

#[tokio::test(flavor = "current_thread")]
async fn a_double_click_returns_the_scales_to_their_fitted_domains() {
    let chart = facets(&["USA", "Japan"], "weight", |_| vec![navigate_x()]);
    let (mut app, _, _) = app(chart).await;
    let fitted = domains(&app, 1);
    let at = center(&app, 1);
    send(&mut app, [moved(at), wheel(2.0)]).await;
    assert_ne!(domains(&app, 1), fitted);

    // The click stands apart from the wheel, so the next press is a double click.
    send(&mut app, double_click(at)).await;
    assert_eq!(domains(&app, 1), fitted);
    assert!(app.state().evaluator().views().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn a_settled_navigation_reports_the_domains_of_its_scales_in_one_change() {
    let navigate = |_: &str| -> Vec<proto::Tool> { vec![proto::Navigate::default().into()] };
    let (mut app, _, changes) = app(facets(&["USA", "Japan"], "weight", navigate)).await;
    let from = center(&app, 0);
    send(&mut app, drag(from, [from[0] - 40.0, from[1] + 30.0])).await;

    let changes = changes.lock().unwrap();
    let [ChartChange { domains: settled }] = &changes[..] else {
        panic!("one change, and not {changes:?}");
    };
    let usa = app.state().plots()[0].identity.clone().unwrap();
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
                second: false,
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
        "the reported domains are the shown domains"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn the_navigation_of_a_scale_bounds_its_view() {
    let navigation = proto::ScaleNavigation {
        bounds: Some(scale_navigation::Bounds::Data(proto::DataBounds {})),
        min_span: Some(500.0),
    };
    let chart = facets_of(
        &["USA"],
        || Scale::new().family("weight").navigation(navigation),
        |_| vec![navigate_x()],
    );
    let (mut app, _, _) = app(chart).await;
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

/// The position of the first car of `origin` in the plot at `plot`.
fn first_car(app: &AvengerApp<ChartAppState>, plot: usize, origin: &str) -> [f32; 2] {
    let plot = &app.state().plots()[plot];
    let PlotCoord::Cartesian {
        x: Some(x),
        y: Some(y),
        ..
    } = &plot.coord
    else {
        panic!("a Cartesian plot with x and y");
    };
    let cars = cars_of(origin);
    let at = |scale: &avenger_scales::scales::ConfiguredScale, column: &str| {
        let value = cars.column_by_name(column).unwrap().slice(0, 1);
        scale.scale_to_numeric(&value).unwrap().as_vec(1, None)[0]
    };
    [plot.rect[0] + at(x, "weight"), plot.rect[1] + at(y, "mpg")]
}

/// The tooltip scene's marks.
fn tooltip_marks(app: &AvengerApp<ChartAppState>) -> &[SceneMark] {
    &app.scenes()[1].marks
}

#[tokio::test(flavor = "current_thread")]
async fn a_tooltip_shows_over_the_plots_that_declare_it() {
    let tooltip = proto::Tooltip {
        find: Some(proto::tooltip::Find::Nearest(proto::NearestFind {
            max_radius: 10.0,
            metric: proto::NearestMetric::Euclidean as i32,
        })),
        ..Default::default()
    };
    let chart = facets(&["USA", "Japan"], "weight", |origin| match origin {
        "USA" => vec![tooltip.clone().into()],
        _ => Vec::new(),
    });
    let (mut app, _, _) = app(chart).await;
    let japan = first_car(&app, 1, "Japan");
    send(&mut app, [moved(japan)]).await;
    assert!(tooltip_marks(&app).is_empty());
    let usa = first_car(&app, 0, "USA");
    send(&mut app, [moved(usa)]).await;
    assert!(!tooltip_marks(&app).is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn a_new_chart_keeps_the_views_of_the_scales_that_it_still_has() {
    let navigate = |_: &str| -> Vec<proto::Tool> { vec![proto::Navigate::default().into()] };
    let (mut app, feed, _) = app(facets(&["USA", "Japan"], "weight", navigate)).await;
    let at = center(&app, 0);
    send(&mut app, [moved(at), wheel(2.0)]).await;
    let zoomed = domains(&app, 0);

    // The family keeps its view, and so does the shared `y`, whose first plot is still USA.
    feed.send(facets(&["USA", "Europe", "Japan"], "weight", navigate));
    app.refresh().await;
    assert_eq!(app.state().plots().len(), 3);
    for plot in 0..3 {
        assert_eq!(domains(&app, plot), zoomed);
    }

    // A chart with another family and other facets keeps neither view.
    feed.send(facets(&["Europe", "Japan"], "mass", navigate));
    app.refresh().await;
    assert!(app.state().evaluator().views().is_empty());
    assert_ne!(domains(&app, 0)[0], zoomed[0]);
}

/// The app shows the newest chart of its task after the request finishes, with the views of the
/// scales that the new chart still has.
#[tokio::test(flavor = "multi_thread")]
async fn a_task_of_charts_shows_its_newest_chart() {
    let navigate = |_: &str| -> Vec<proto::Tool> { vec![proto::Navigate::default().into()] };
    let task: Task<usize, Chart> = Task::new();
    let mut app = ChartApp::new(facets(&["USA", "Japan"], "weight", navigate), engine())
        .size(SIZE)
        .charts(&task)
        .build()
        .await
        .unwrap()
        .with_runtime(tokio::runtime::Handle::current())
        .unwrap();
    let at = center(&app, 0);
    send(&mut app, [moved(at), wheel(2.0)]).await;
    let zoomed = domains(&app, 0);

    let three = facets(&["USA", "Europe", "Japan"], "weight", navigate);
    task.request(3, move |_| async move {
        Ok::<_, std::convert::Infallible>(three)
    });
    for _ in 0..500 {
        app.refresh().await;
        if app.state().plots().len() == 3 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(app.state().plots().len(), 3);
    for plot in 0..3 {
        assert_eq!(domains(&app, plot), zoomed);
    }
}
