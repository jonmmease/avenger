//! The chart's drawing, and its selections driven by window events: a click on a bar, a box, and
//! a click that clears the box.

use std::time::Duration;

use avenger_app::app::AvengerApp;
use avenger_chart_app::ChartAppState;
use avenger_chart_eval::{ChartEvaluator, Theme};
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

/// The position at `fraction` of the width and the height of the plot named `id`.
fn inside(app: &AvengerApp<ChartAppState>, id: &str, fraction: [f32; 2]) -> [f32; 2] {
    let plot = (app.state().plots().iter())
        .find(|plot| (plot.identity.as_ref()).is_some_and(|identity| identity.id == id))
        .expect("the plot");
    let [x, y, width, height] = plot.rect;
    [x + width * fraction[0], y + height * fraction[1]]
}

/// The number of contributions of the selection `name`.
fn contributions(app: &AvengerApp<ChartAppState>, name: &str) -> usize {
    (app.state().evaluator().selections().get(name)).map_or(0, Vec::len)
}

/// The chart evaluates without warnings at two sizes, with its three plots.
#[test]
fn the_chart_draws_its_three_plots_at_two_sizes() {
    let mut evaluator =
        ChartEvaluator::new(&chart(&movies()), &engine(), &Theme::default()).unwrap();
    for size in [SIZE, [600.0, 450.0]] {
        let evaluated = evaluator.evaluate(size).unwrap();
        assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
        assert_eq!(evaluated.plots.len(), 3);
    }
}

/// A click on the first bar selects its genre, a box writes the brush, and a click in the plot
/// of the box clears the box and keeps the genre.
#[tokio::test(flavor = "current_thread")]
async fn clicks_and_boxes_write_their_selections() {
    let chart = chart(&movies());
    let mut app = ChartApp::new(chart, engine())
        .size(SIZE)
        .build()
        .await
        .unwrap();
    let drama = inside(&app, "genres", [1.0 / 14.0, 0.95]);
    send(&mut app, click(drama)).await;
    assert_eq!(contributions(&app, GENRES), 1);
    let from = inside(&app, "ratings", [0.3, 0.3]);
    let to = inside(&app, "ratings", [0.7, 0.7]);
    send(&mut app, drag(from, to)).await;
    assert_eq!(contributions(&app, BRUSH), 1);
    std::thread::sleep(Duration::from_millis(400));
    let away = inside(&app, "ratings", [0.95, 0.05]);
    send(&mut app, click(away)).await;
    assert_eq!(contributions(&app, BRUSH), 0);
    assert_eq!(contributions(&app, GENRES), 1);
}
