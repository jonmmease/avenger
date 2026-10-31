//! The chart's drawing, and its linked selection driven by window events: a box in the overview,
//! a pan of the detail, and a double click on the detail.

use std::time::Duration;

use avenger_app::app::AvengerApp;
use avenger_chart_app::ChartAppState;
use avenger_chart_eval::{ChartEvaluator, PlotCoord, Theme};
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

fn double_click(at: [f32; 2]) -> [WindowEvent; 5] {
    [
        moved(at),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
    ]
}

/// The position at `fraction` of the width and the height of the plot at `plot`.
fn inside(app: &AvengerApp<ChartAppState>, plot: usize, fraction: [f32; 2]) -> [f32; 2] {
    let [x, y, width, height] = app.state().plots()[plot].rect;
    [x + width * fraction[0], y + height * fraction[1]]
}

/// The domain of the detail's years.
fn detail_years(app: &AvengerApp<ChartAppState>) -> (f64, f64) {
    let PlotCoord::Cartesian { x: Some(x), .. } = &app.state().plots()[0].coord else {
        panic!("a Cartesian detail with years on x");
    };
    x.numeric_interval_domain().unwrap()
}

/// The years of the box, when the selection has one.
fn focused(app: &AvengerApp<ChartAppState>) -> Option<[f64; 2]> {
    years_of(app.state().evaluator().selections().get(FOCUS)?)
}

/// The chart evaluates without warnings at two sizes, with the detail above the overview.
#[test]
fn the_chart_draws_its_two_plots_at_two_sizes() {
    let mut evaluator =
        ChartEvaluator::new(&chart(&movies()), &engine(), &Theme::default()).unwrap();
    for size in [SIZE, [600.0, 450.0]] {
        let evaluated = evaluator.evaluate(size).unwrap();
        assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
        assert_eq!(evaluated.plots.len(), 2);
        assert!(evaluated.plots[0].rect[3] > 2.0 * evaluated.plots[1].rect[3]);
    }
}

/// A box in the overview sets the years of the detail, a pan of the detail moves the box, and a
/// double click on the detail clears it.
#[tokio::test(flavor = "current_thread")]
async fn the_box_and_the_detail_move_together() {
    let mut app = ChartApp::new(chart(&movies()), engine())
        .size(SIZE)
        .build()
        .await
        .unwrap();
    let every_year = detail_years(&app);
    let from = inside(&app, 1, [0.5, 0.5]);
    let to = inside(&app, 1, [0.8, 0.5]);
    send(&mut app, drag(from, to)).await;
    let [low, high] = focused(&app).expect("a box");
    assert_eq!(detail_years(&app), (low, high));

    let from = inside(&app, 0, [0.5, 0.5]);
    let to = inside(&app, 0, [0.3, 0.5]);
    send(&mut app, drag(from, to)).await;
    let [panned_low, panned_high] = focused(&app).expect("a box");
    assert!(panned_low > low, "{panned_low} after {low}");
    assert!((panned_high - panned_low - (high - low)).abs() < 1e-6);
    assert_eq!(detail_years(&app), (panned_low, panned_high));

    std::thread::sleep(Duration::from_millis(400));
    let at = inside(&app, 0, [0.5, 0.5]);
    send(&mut app, double_click(at)).await;
    assert_eq!(focused(&app), None);
    assert_eq!(detail_years(&app), every_year);
}
