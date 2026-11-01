//! The dashboards at two sizes, and the pipeline driven by window events: a box on the overview
//! that settles a window, the task that loads its dashboard, and a double click on the detail
//! that clears it.

use std::collections::BTreeMap;
use std::time::Duration;

use arrow::array::Date64Array;
use arrow::datatypes::Date64Type;
use avenger_app::app::AvengerApp;
use avenger_chart_app::ChartAppState;
use avenger_chart_eval::{ChartEvaluator, EvaluatedPlot, PlotCoord, Theme};
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};

use super::*;

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

fn click(at: [f32; 2]) -> [WindowEvent; 3] {
    [
        moved(at),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
    ]
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

fn plot<'a>(app: &'a AvengerApp<ChartAppState>, id: &str) -> &'a EvaluatedPlot {
    (app.state().plots().iter())
        .find(|plot| (plot.identity.as_ref()).is_some_and(|identity| identity.id == id))
        .expect("the plot")
}

/// The position at `fraction` of the width and the height of the first plot named `id`.
fn inside(app: &AvengerApp<ChartAppState>, id: &str, fraction: [f32; 2]) -> [f32; 2] {
    let [x, y, width, height] = plot(app, id).rect;
    [x + width * fraction[0], y + height * fraction[1]]
}

/// The domain of the dates of the first plot named `id`, in milliseconds since the epoch.
fn date_domain(app: &AvengerApp<ChartAppState>, id: &str) -> (f64, f64) {
    let PlotCoord::Cartesian { x: Some(x), .. } = &plot(app, id).coord else {
        panic!("a Cartesian plot with x");
    };
    x.numeric_interval_domain().unwrap()
}

/// The contributions of the window.
fn window_contributions(app: &AvengerApp<ChartAppState>) -> Vec<Contribution> {
    (app.state().evaluator().selections().get(WINDOW))
        .cloned()
        .unwrap_or_default()
}

/// The ends of the range of the window, in milliseconds since the epoch.
fn window_ends(contributions: &[Contribution]) -> (f64, f64) {
    let ends = (contributions.iter())
        .flat_map(|contribution| contribution.tuples.iter().flatten())
        .find_map(|test| match test {
            TupleTest::Range { ends, .. } => Some(ends),
            _ => None,
        })
        .expect("a range of dates");
    let ends = cast(ends, &DataType::Date64).unwrap();
    let ends = ends.as_primitive::<Date64Type>();
    (ends.value(0) as f64, ends.value(1) as f64)
}

/// Whether the app shows `chart`.
fn shows(app: &AvengerApp<ChartAppState>, chart: &Chart) -> bool {
    let shown = app.state().evaluator().chart();
    shown.message() == chart.message() && shown.tables() == chart.tables()
}

/// Refreshes the app until it shows `chart`, which the task loads.
async fn wait_for(app: &mut AvengerApp<ChartAppState>, chart: &Chart) {
    for _ in 0..1000 {
        app.refresh().await;
        if shows(app, chart) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the app shows the loaded dashboard");
}

/// The dashboard of every date and the dashboard of a window evaluate without warnings at two
/// sizes, with the detail, a plot for each sector, and the overview.
#[test]
fn the_dashboards_draw_their_plots_at_two_sizes() {
    let loader = Loader::new(Prices::generate(SEED));
    let from = NaiveDate::from_ymd_opt(2016, 3, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2016, 9, 30).unwrap();
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    let ends = [from, to].map(|date| (date - epoch).num_milliseconds());
    let window = Contribution::new("overview").tuple([TupleTest::Range {
        column: DATE.into(),
        channel: Channel::X,
        ends: Arc::new(Date64Array::from(ends.to_vec())),
        exclude_end: false,
    }]);
    for (window, contributions) in [(None, vec![]), (Some([from, to]), vec![window])] {
        let load = loader.load(window);
        let mut evaluator = ChartEvaluator::new(&load.chart, &engine(), &Theme::default()).unwrap();
        evaluator
            .set_selections(BTreeMap::from([(WINDOW.to_string(), contributions)]))
            .unwrap();
        for size in [SIZE, [800.0, 600.0]] {
            let evaluated = evaluator.evaluate(size).unwrap();
            assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
            assert_eq!(evaluated.plots.len(), 2 + SECTORS.len());
        }
    }
    assert_eq!(
        loader.load(None).resolution,
        Resolution::Month,
        "every date draws by month"
    );
}

/// A box on the overview settles a window. The task loads the daily or weekly rows around the
/// window, the app shows the new dashboard with the window in place, and the detail shows the
/// dates of the window. A double click on the detail clears the window, and the task loads the
/// monthly rows of every date.
#[tokio::test(flavor = "multi_thread")]
async fn a_settled_window_loads_a_dashboard_on_the_task() {
    let loader = Arc::new(Loader::new(Prices::generate(SEED)));
    let dashboards = Dashboards::new();
    let mut app = ChartApp::new(loader.load(None).chart, engine())
        .size(SIZE)
        .charts(&dashboards)
        .on_change(on_change(loader.clone(), dashboards.clone()))
        .build()
        .await
        .unwrap()
        .with_runtime(tokio::runtime::Handle::current())
        .unwrap();

    let from = inside(&app, "overview", [0.40, 0.5]);
    let to = inside(&app, "overview", [0.48, 0.5]);
    send(&mut app, drag(from, to)).await;
    let contributions = window_contributions(&app);
    assert_eq!(contributions.len(), 1);
    let window = window_of(&contributions).expect("a window");
    let load = loader.load(Some(window));
    assert_ne!(load.resolution, Resolution::Month);
    wait_for(&mut app, &load.chart).await;
    assert_eq!(
        window_ends(&window_contributions(&app)),
        window_ends(&contributions)
    );
    let (start, end) = date_domain(&app, "detail");
    let (from, to) = window_ends(&contributions);
    assert!((start - from).abs() < 1.0 && (end - to).abs() < 1.0);
    assert_eq!(date_domain(&app, SECTOR), (start, end));

    std::thread::sleep(Duration::from_millis(400));
    let at = inside(&app, "detail", [0.5, 0.5]);
    send(&mut app, click(at)).await;
    send(&mut app, click(at)).await;
    assert!(window_contributions(&app).is_empty());
    wait_for(&mut app, &loader.load(None).chart).await;
}
