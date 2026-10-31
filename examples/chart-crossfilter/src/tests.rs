//! The chart's drawing, and the cross filter driven by window events: a box and a range of years
//! that fade the movies of the other plots, and a click that clears the box.

use std::time::Duration;

use avenger_app::app::AvengerApp;
use avenger_chart_app::ChartAppState;
use avenger_chart_eval::{ChartEvaluator, EvaluatedPlot, Theme};
use avenger_color::ColorOrGradient;
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_scenegraph::marks::{mark::SceneMark, symbol::SceneSymbolMark};

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
    let [x, y, width, height] = plot(app, id).rect;
    [x + width * fraction[0], y + height * fraction[1]]
}

fn plot<'a>(app: &'a AvengerApp<ChartAppState>, id: &str) -> &'a EvaluatedPlot {
    (app.state().plots().iter())
        .find(|plot| (plot.identity.as_ref()).is_some_and(|identity| identity.id == id))
        .expect("the plot")
}

/// The plots that hold a part of the selection `filter`, in their order in the window.
fn owners(app: &AvengerApp<ChartAppState>) -> Vec<String> {
    let contributions = app.state().evaluator().selections().get(FILTER);
    let mut owners: Vec<String> = (contributions.into_iter().flatten())
        .map(|contribution| contribution.owner.clone())
        .collect();
    let order = ["ratings", "imdb_by_year", "tomatoes_by_year"];
    owners.sort_by_key(|owner| order.iter().position(|id| id == owner));
    owners
}

/// The symbols of the group named `name` among `marks` and the groups inside them.
fn symbols<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneSymbolMark> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Group(group) if group.name == name => {
            group.marks.iter().find_map(|mark| match mark {
                SceneMark::Symbol(symbol) => Some(symbol),
                _ => None,
            })
        }
        SceneMark::Group(group) => symbols(&group.marks, name),
        _ => None,
    })
}

/// Whether each movie of the plot named `id` is opaque, in the order of the movies.
fn opaque(app: &AvengerApp<ChartAppState>, id: &str) -> Vec<bool> {
    let symbol = symbols(&app.scenes()[0].marks, &plot(app, id).name).expect("the symbols");
    (symbol.fill.as_vec(symbol.len as usize, None).into_iter())
        .map(|fill| matches!(fill, ColorOrGradient::Color(color) if color[3] > 0.2))
        .collect()
}

/// Whether some movies of `opaque` are opaque and others are faded.
fn mixed(opaque: &[bool]) -> bool {
    opaque.iter().any(|opaque| *opaque) && opaque.iter().any(|opaque| !opaque)
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

/// A box on the ratings and a range of years each fade the movies of the other plots but none
/// of their own, the plot without a part fades the movies outside both, and a click clears the
/// box and keeps the range.
#[tokio::test(flavor = "current_thread")]
async fn each_plot_fades_the_movies_outside_the_parts_of_the_others() {
    let mut app = ChartApp::new(chart(&movies()), engine())
        .size(SIZE)
        .build()
        .await
        .unwrap();
    let from = inside(&app, "ratings", [0.3, 0.2]);
    let to = inside(&app, "ratings", [0.8, 0.6]);
    send(&mut app, drag(from, to)).await;
    assert_eq!(owners(&app), ["ratings"]);
    assert!(opaque(&app, "ratings").iter().all(|opaque| *opaque));
    let in_box = opaque(&app, "imdb_by_year");
    assert!(mixed(&in_box));
    assert_eq!(opaque(&app, "tomatoes_by_year"), in_box);

    std::thread::sleep(Duration::from_millis(400));
    let from = inside(&app, "imdb_by_year", [0.5, 0.5]);
    let to = inside(&app, "imdb_by_year", [0.9, 0.5]);
    send(&mut app, drag(from, to)).await;
    assert_eq!(owners(&app), ["ratings", "imdb_by_year"]);
    let in_range = opaque(&app, "ratings");
    assert!(mixed(&in_range));
    assert_eq!(opaque(&app, "imdb_by_year"), in_box);
    let in_both: Vec<bool> = (in_box.iter().zip(&in_range))
        .map(|(in_box, in_range)| *in_box && *in_range)
        .collect();
    assert!(mixed(&in_both));
    assert_eq!(opaque(&app, "tomatoes_by_year"), in_both);

    std::thread::sleep(Duration::from_millis(400));
    let away = inside(&app, "ratings", [0.05, 0.95]);
    send(&mut app, click(away)).await;
    assert_eq!(owners(&app), ["imdb_by_year"]);
    assert!(opaque(&app, "imdb_by_year").iter().all(|opaque| *opaque));
    assert_eq!(opaque(&app, "tomatoes_by_year"), in_range);
}
