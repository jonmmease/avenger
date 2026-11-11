//! The maps and their modes: the mode widgets above the maps, the drags that pan in the Pan mode,
//! draw a box in the Box mode, and draw a lasso in the Lasso mode, and the check box of the
//! tooltips. The tests draw no tiles, since the app has no image cache.

use std::sync::Mutex;

use avenger_chart_app::{ChartAppState, WidgetSpec};
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};
use avenger_selection::Condition;

use super::*;

type App = avenger_app::app::AvengerApp<ChartAppState>;

/// The app of the chart, with each settled change in `heard`.
async fn app(heard: Arc<Mutex<Vec<ChartChange>>>) -> App {
    ChartApp::new(chart(), cascade(), engine())
        .size(SIZE)
        .reports(Reports::Settled)
        .on_change(move |change| heard.lock().unwrap().push(change.clone()))
        .build()
        .await
        .unwrap()
        .with_runtime(tokio::runtime::Handle::current())
        .unwrap()
}

/// Sends `events` to the app, apart in time.
async fn send(app: &mut App, events: impl IntoIterator<Item = WindowEvent>) {
    for event in events {
        std::thread::sleep(Duration::from_millis(10));
        app.update(&event, Instant::now()).await;
    }
}

fn moved(at: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position: at })
}

fn mouse(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

/// A press at the first of `path`, moves through the rest, and a release at the last.
fn drag_through(path: &[[f32; 2]]) -> Vec<WindowEvent> {
    let mut events = vec![moved(path[0]), mouse(ElementState::Pressed)];
    events.extend(path[1..].iter().map(|at| moved(*at)));
    events.push(mouse(ElementState::Released));
    events
}

/// The position of the text `text` among `marks`, inside groups at `origin`.
fn text_in(marks: &[SceneMark], origin: [f32; 2], text: &str) -> Option<[f32; 2]> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Group(group) => text_in(
            &group.marks,
            [origin[0] + group.origin[0], origin[1] + group.origin[1]],
            text,
        ),
        SceneMark::Text(mark) if mark.text.as_vec(1, None)[0] == text => {
            let [x, y] = [mark.x.as_vec(1, None)[0], mark.y.as_vec(1, None)[0]];
            Some([origin[0] + x, origin[1] + y])
        }
        _ => None,
    })
}

/// Clicks the label `text` of the mode widget, in the app's highest scene.
async fn click_mode(app: &mut App, text: &str) {
    let at = {
        let scenes = app.scenes();
        let scene: &SceneGraph = &scenes[scenes.len() - 1];
        text_in(&scene.marks, scene.origin, text).unwrap_or_else(|| panic!("the label {text}"))
    };
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

/// The point at the fractions `[x, y]` of the rectangle of the plot `index`.
fn at(app: &App, index: usize, [x, y]: [f32; 2]) -> [f32; 2] {
    let [left, top, width, height] = app.state().plots()[index].rect;
    [left + width * x, top + height * y]
}

/// The clauses of the selection of the earthquakes.
fn quakes(app: &App) -> Vec<avenger_selection::Clause> {
    app.state().evaluator().selections()[QUAKES].clauses.clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_widgets_of_the_modes_stand_in_a_row_above_the_maps() {
    let app = app(Arc::default()).await;
    let plots = app.state().plots();
    assert_eq!(plots.len(), 2);
    let [drag, tooltips] = app.state().widgets() else {
        panic!("two widgets: {:#?}", app.state().widgets());
    };
    let WidgetSpec::ModeGroup { modes, labels } = &drag.spec else {
        panic!("a widget of a group: {:?}", drag.spec);
    };
    assert_eq!(modes, &["pan", "box", "lasso"]);
    let labels: Vec<_> = labels.iter().map(|label| label.source()).collect();
    assert_eq!(labels, ["Pan", "Box", "Lasso"]);
    assert!(matches!(&tooltips.spec, WidgetSpec::ModeToggle { mode } if mode == "tooltips"));
    // The maps share each widget, which draws once, above both, and the check box stands beside
    // the radio group.
    let ([drag], [tooltips]) = (&drag.rects[..], &tooltips.rects[..]) else {
        panic!("one drawing of each: {drag:?} {tooltips:?}");
    };
    assert!(tooltips[0] >= drag[0] + drag[2], "{drag:?} {tooltips:?}");
    for plot in plots {
        assert!(
            drag[1] + drag[3] <= plot.rect[1],
            "{drag:?} {:?}",
            plot.rect
        );
    }
    assert_eq!(app.state().modes(), ["pan", "tooltips"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_check_box_turns_the_tooltips_off_and_on() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut app = app(heard.clone()).await;
    click_mode(&mut app, "Tooltips").await;
    assert_eq!(app.state().modes(), ["pan"]);
    let last = heard.lock().unwrap().last().cloned();
    assert_eq!(
        last.and_then(|change| change.modes),
        Some(vec!["pan".to_string()])
    );
    std::thread::sleep(Duration::from_millis(600));
    click_mode(&mut app, "Tooltips").await;
    assert_eq!(app.state().modes(), ["pan", "tooltips"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drag_pans_a_map_in_the_pan_mode_and_draws_a_box_in_the_box_mode() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut app = app(heard.clone()).await;
    let (from, to) = (at(&app, 1, [0.3, 0.3]), at(&app, 1, [0.6, 0.6]));
    send(&mut app, drag_through(&[from, to])).await;
    assert!(app.state().evaluator().projection_view(1).is_some());
    assert!(quakes(&app).is_empty());

    // The next press stands apart from the last release, so it is no double click.
    std::thread::sleep(Duration::from_millis(600));
    click_mode(&mut app, "Box").await;
    assert_eq!(app.state().modes(), ["box", "tooltips"]);
    let last = heard.lock().unwrap().last().cloned();
    let modes = last.and_then(|change| change.modes);
    assert_eq!(modes, Some(vec!["box".to_string(), "tooltips".to_string()]));
    let view = |app: &App| format!("{:?}", app.state().evaluator().projection_view(1));
    let panned = view(&app);
    std::thread::sleep(Duration::from_millis(600));
    send(&mut app, drag_through(&[from, to])).await;
    // The box selects ranges of longitude and latitude, and pans nothing.
    let clauses = quakes(&app);
    let [clause] = &clauses[..] else {
        panic!("one clause: {clauses:?}");
    };
    let columns: Vec<&str> = (clause.conditions.iter())
        .flat_map(|condition| condition.columns())
        .collect();
    assert_eq!(columns, ["lon", "lat"]);
    assert_eq!(view(&app), panned);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drag_draws_a_lasso_in_the_lasso_mode() {
    let mut app = app(Arc::default()).await;
    click_mode(&mut app, "Lasso").await;
    assert_eq!(app.state().modes(), ["lasso", "tooltips"]);
    let path = [[0.3, 0.3], [0.6, 0.3], [0.6, 0.6], [0.3, 0.6], [0.31, 0.31]]
        .map(|fraction| at(&app, 0, fraction));
    std::thread::sleep(Duration::from_millis(600));
    send(&mut app, drag_through(&path)).await;
    let clauses = quakes(&app);
    let [clause] = &clauses[..] else {
        panic!("one clause: {clauses:?}");
    };
    assert!(
        matches!(clause.conditions[..], [Condition::Polygon { .. }]),
        "{:?}",
        clause.conditions
    );
}
