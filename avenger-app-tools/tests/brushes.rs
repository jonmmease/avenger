//! Interval brushes draw, move, resize, clear, and cancel boxes in keyed areas, beside panning.
//! Point brushes select instances by their keys, and lasso brushes select the instances inside
//! their paths.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arrow_array::cast::AsArray;
use arrow_array::types::Int64Type;
use arrow_array::{ArrayRef, Int64Array, RecordBatch};
use async_trait::async_trait;
use avenger_app::app::{AppBuild, AppBuilder, AvengerApp, BuiltScene, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app_tools::brushes::{
    Aspect, Brush, BrushStyle, Brushes, Instances, Interval, KeyedIntervalBrush, KeyedLassoBrush,
    KeyedPointBrush, Lasso, PointBrush,
};
use avenger_app_tools::navigation::PanZoom;
use avenger_color::ColorOrGradient;
use avenger_common::cursor::CursorIcon;
use avenger_common::time::Instant;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_eventstream::stream::Rebuild;
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, MouseScrollDelta, NamedKey, WindowCursorMoved, WindowEvent,
    WindowKeyboardInput, WindowMouseInput, WindowMouseWheel, WindowResizeEvent,
};
use avenger_geometry::data::{Datum, SceneData};
use avenger_layout::Rect;
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::scene_graph::SceneGraph;

const PLOT: &str = "plot";
const BRUSHES: &str = "brushes";

/// The panels' plot rectangles. Each panel's x scale spans its width, and its y scale increases
/// upward.
const LEFT: Rect = Rect::new(50.0, 50.0, 200.0, 100.0);
const RIGHT: Rect = Rect::new(300.0, 50.0, 200.0, 100.0);

/// A box from `[70, 70]` to `[150, 130]` in the left panel.
const BOX: Interval = Interval {
    x: Some([10.0, 50.0]),
    y: Some([2.0, 8.0]),
};

struct Chart {
    x: ConfiguredScale,
    y: ConfiguredScale,
    panels: Vec<(u8, Rect)>,
    /// Whether the brushes claim presses.
    select: bool,
    /// One selection for the figure, in the panel where it was drawn.
    selection: Option<(u8, Interval)>,
    /// One selection for each panel.
    boxes: HashMap<u8, Interval>,
    changes: Vec<(u8, Option<Interval>)>,
    settles: Vec<(u8, Option<Interval>)>,
    /// The keys of the instances that the point brush selected.
    picked: Vec<i64>,
    /// The keys of the instances that the keyed point brush selected in each panel.
    panel_picks: HashMap<u8, Vec<i64>>,
    /// The marks whose instances a point brush with instances from state selects.
    pickable: Vec<&'static str>,
    /// Each panel's lasso.
    lassos: HashMap<u8, Lasso<i64>>,
}

impl Default for Chart {
    fn default() -> Self {
        Self {
            x: LinearScale::configured((0.0, 100.0), (0.0, 200.0)),
            y: LinearScale::configured((0.0, 10.0), (100.0, 0.0)),
            panels: vec![(0, LEFT), (1, RIGHT)],
            select: true,
            selection: None,
            boxes: HashMap::new(),
            changes: Vec::new(),
            settles: Vec::new(),
            pickable: vec![DOTS.0, RINGS.0],
            picked: Vec::new(),
            panel_picks: HashMap::new(),
            lassos: HashMap::new(),
        }
    }
}

/// Dots and rings in the right panel, each mark with its own rows and IDs, as an app attaches
/// filtered batches: `(mark, centers, ids)`.
const DOTS: (&str, [[f32; 2]; 3], [i64; 3]) = (
    "dots",
    [[350.0, 100.0], [400.0, 100.0], [450.0, 100.0]],
    [10, 11, 12],
);
const RINGS: (&str, [[f32; 2]; 2], [i64; 2]) =
    ("rings", [[350.0, 130.0], [400.0, 130.0]], [20, 21]);

struct Plot;

#[async_trait]
impl AppBuild<Chart> for Plot {
    fn scenes(&self) -> Vec<Scene<Chart>> {
        vec![Scene::new(PLOT, 0)]
    }

    async fn build(
        &self,
        _state: &mut Chart,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(PLOT) {
            return Ok(());
        }
        let mut marks = Vec::new();
        let mut data = SceneData::new();
        let mut add = |name: &str, centers: &[[f32; 2]], ids: &[i64]| {
            let symbols = SceneSymbolMark {
                name: name.to_string(),
                len: centers.len() as u32,
                x: ScalarOrArray::new_array(centers.iter().map(|c| c[0]).collect()),
                y: ScalarOrArray::new_array(centers.iter().map(|c| c[1]).collect()),
                shapes: vec![SymbolShape::Circle],
                size: 100.0.into(),
                ..Default::default()
            };
            marks.push(symbols.into());
            let ids: ArrayRef = Arc::new(Int64Array::from(ids.to_vec()));
            data.insert(
                name,
                Arc::new(RecordBatch::try_from_iter([("id", ids)]).unwrap()),
            );
        };
        add(DOTS.0, &DOTS.1, &DOTS.2);
        add(RINGS.0, &RINGS.1, &RINGS.2);
        let [width, height] = scenes.size();
        let scene = SceneGraph {
            marks,
            width,
            height,
            origin: [0.0; 2],
        };
        scenes.set(PLOT, BuiltScene { scene, data })
    }
}

/// A brush over every panel with one selection for the figure.
fn figure() -> KeyedIntervalBrush<Chart, u8> {
    KeyedIntervalBrush::xy("select")
        .areas(|s: &Chart| s.panels.clone())
        .x_scale(|s, _| s.x.clone())
        .y_scale(|s, _| s.y.clone())
        .value(|s, panel| (s.selection).filter(|(at, _)| at == panel).map(|(_, i)| i))
        .on_change(|s, panel, interval| {
            s.selection = interval.map(|interval| (*panel, interval));
            s.changes.push((*panel, interval));
            Rebuild::scene(PLOT)
        })
        .on_settle(|s, panel, interval| {
            s.settles.push((*panel, interval));
            Rebuild::tree(PLOT)
        })
        .active(|s| s.select)
}

/// A brush over every panel with one selection for each panel.
fn per_panel() -> KeyedIntervalBrush<Chart, u8> {
    KeyedIntervalBrush::xy("panels")
        .areas(|s: &Chart| s.panels.clone())
        .x_scale(|s, _| s.x.clone())
        .y_scale(|s, _| s.y.clone())
        .value(|s, panel| s.boxes.get(panel).copied())
        .on_change(|s, panel, interval| {
            match interval {
                Some(interval) => s.boxes.insert(*panel, interval),
                None => s.boxes.remove(panel),
            };
            Rebuild::scene(PLOT)
        })
}

/// The ID in an instance's row.
fn id(datum: &Datum) -> Option<i64> {
    let ids = datum.rows.column_by_name("id")?.as_primitive::<Int64Type>();
    Some(ids.value(datum.row))
}

/// A brush that selects dots and rings by their IDs.
fn picks() -> PointBrush<Chart, i64> {
    PointBrush::new(Instances::marks([DOTS.0, RINGS.0]).key(id))
        .value(|s: &Chart| s.picked.clone())
        .on_change(|s, picked| {
            s.picked = picked;
            Rebuild::tree(PLOT)
        })
}

/// A brush that selects dots and rings by their IDs in each panel, with one selection for each
/// panel. The left panel has no instances.
fn panel_picks() -> KeyedPointBrush<Chart, u8, i64> {
    KeyedPointBrush::new()
        .areas(|s: &Chart| s.panels.clone())
        .instances(|_, panel| match panel {
            1 => Instances::marks([DOTS.0, RINGS.0]).key(id),
            _ => Instances::marks(Vec::<String>::new()),
        })
        .value(|s, panel| s.panel_picks.get(panel).cloned().unwrap_or_default())
        .on_change(|s, panel, picked| {
            s.panel_picks.insert(*panel, picked);
            Rebuild::tree(PLOT)
        })
}

/// A brush that lassos dots and rings in every panel, with one lasso for each panel. Each new
/// vertex rebuilds the plot without its tree, as a highlight of the lasso would, and the release
/// rebuilds the tree.
fn lassos() -> KeyedLassoBrush<Chart, u8, i64> {
    KeyedLassoBrush::new("lasso")
        .areas(|s: &Chart| s.panels.clone())
        .x_scale(|s, _| s.x.clone())
        .y_scale(|s, _| s.y.clone())
        .instances(Instances::marks([DOTS.0, RINGS.0]).key(id))
        .value(|s, panel| s.lassos.get(panel).cloned().unwrap_or_default())
        .on_change(|s, panel, lasso| {
            s.lassos.insert(*panel, lasso);
            Rebuild::scene(PLOT)
        })
        .on_settle(|_, _, _| Rebuild::tree(PLOT))
}

async fn app(tools: impl FnOnce(AppBuilder<Chart>) -> AppBuilder<Chart>) -> AvengerApp<Chart> {
    let engine = avenger_typst_label::LabelEngine::new(Default::default());
    let builder = AvengerApp::builder(Chart::default(), Plot, engine).size([600.0, 300.0]);
    tools(builder).build().await.unwrap()
}

async fn brushed(brush: KeyedIntervalBrush<Chart, u8>) -> AvengerApp<Chart> {
    brushed_with(brush).await
}

async fn brushed_with(brush: impl Into<Brush<Chart>>) -> AvengerApp<Chart> {
    app(|builder| builder.tool(Brushes::new(BRUSHES, 10).with(brush))).await
}

fn pan_zoom() -> PanZoom<Chart> {
    PanZoom::xy(PLOT, |_| LEFT, |s: &mut Chart| &mut s.x, |s| &mut s.y)
}

/// Send events 10 ms apart so the pan's 8 ms throttle accepts each move.
async fn send(app: &mut AvengerApp<Chart>, events: impl IntoIterator<Item = WindowEvent>) {
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

fn click(at: [f32; 2]) -> [WindowEvent; 3] {
    [
        moved(at),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
    ]
}

fn key(key: NamedKey, state: ElementState) -> WindowEvent {
    WindowEvent::KeyboardInput(WindowKeyboardInput {
        key: Key::Named(key),
        state,
        text: None,
    })
}

fn escape() -> WindowEvent {
    key(NamedKey::Escape, ElementState::Pressed)
}

/// A click at `at` with Shift held.
fn shift_click(at: [f32; 2]) -> Vec<WindowEvent> {
    let mut events = vec![key(NamedKey::Shift, ElementState::Pressed)];
    events.extend(click(at));
    events.push(key(NamedKey::Shift, ElementState::Released));
    events
}

/// A drag through `corners` that moves 10 pixels at a time and releases near the first corner.
fn trace(corners: &[[f32; 2]]) -> Vec<WindowEvent> {
    let mut events = vec![moved(corners[0]), mouse(ElementState::Pressed)];
    for (from, to) in corners
        .iter()
        .zip(corners.iter().skip(1).chain([&corners[0]]))
    {
        let steps = ((to[0] - from[0]).hypot(to[1] - from[1]) / 10.0).ceil() as usize;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            events.push(moved([
                from[0] + t * (to[0] - from[0]),
                from[1] + t * (to[1] - from[1]),
            ]));
        }
    }
    events.push(mouse(ElementState::Released));
    events
}

fn wheel() -> WindowEvent {
    WindowEvent::MouseWheel(WindowMouseWheel {
        delta: MouseScrollDelta::LineDelta(0.0, 1.0),
    })
}

/// Rebuild every scene, as a resize does.
fn rebuild() -> WindowEvent {
    WindowEvent::WindowResize(WindowResizeEvent {
        size: [600.0, 300.0],
    })
}

/// The rect and path marks named `name` in the brushes' scene, each with its group's origin.
fn marks<'a>(
    app: &'a AvengerApp<Chart>,
    name: &'a str,
) -> impl Iterator<Item = ([f32; 2], &'a SceneMark)> + 'a {
    let groups = app.scenes()[1].marks.iter().filter_map(|mark| match mark {
        SceneMark::Group(group) => Some(group),
        _ => None,
    });
    groups.flat_map(move |group| {
        let named = move |mark: &&SceneMark| match mark {
            SceneMark::Rect(rect) => rect.name == name,
            SceneMark::Path(path) => path.name == name,
            _ => false,
        };
        group
            .marks
            .iter()
            .filter(named)
            .map(|mark| (group.origin, mark))
    })
}

/// The path of the first path mark named `name` in the brushes' scene.
fn path(app: &AvengerApp<Chart>, name: &str) -> lyon_path::Path {
    let path = marks(app, name).find_map(|(_, mark)| match mark {
        SceneMark::Path(path) => path.path.first().cloned(),
        _ => None,
    });
    path.unwrap_or_else(|| panic!("the brushes draw no path named {name:?}"))
}

/// The rectangle of each drawn box, `[x, y, width, height]` in scene coordinates, in drawing order.
fn boxes(app: &AvengerApp<Chart>) -> Vec<[f32; 4]> {
    let boxes = marks(app, "brush_box").filter_map(|([x, y], mark)| match mark {
        SceneMark::Rect(rect) => Some([
            x + rect.x_vec()[0],
            y + rect.y_vec()[0],
            rect.width_vec()[0],
            rect.height_vec()[0],
        ]),
        _ => None,
    });
    boxes.collect()
}

/// The pixel of `value` through `scale`, from the start of the scale's range.
fn pixel(scale: &ConfiguredScale, value: f64) -> f32 {
    let values: ArrayRef = Arc::new(arrow_array::Float64Array::from(vec![value]));
    *scale.scale_to_numeric(&values).unwrap().first().unwrap()
}

/// Point picks over a pan of the right panel.
async fn picks_over_pan() -> AvengerApp<Chart> {
    let pan = PanZoom::xy(PLOT, |_| RIGHT, |s: &mut Chart| &mut s.x, |s| &mut s.y);
    let brushes = Brushes::new(BRUSHES, 10).with(picks());
    app(|builder| builder.tool(pan).tool(brushes)).await
}

/// Whether `a` and `b` select the same ranges, up to rounding.
fn same(a: Option<Interval>, b: Option<Interval>) -> bool {
    let axis = |a: Option<[f64; 2]>, b: Option<[f64; 2]>| match (a, b) {
        (Some(a), Some(b)) => (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6,
        (a, b) => a.is_none() && b.is_none(),
    };
    match (a, b) {
        (Some(a), Some(b)) => axis(a.x, b.x) && axis(a.y, b.y),
        (a, b) => a.is_none() && b.is_none(),
    }
}

fn selection(app: &AvengerApp<Chart>) -> Option<Interval> {
    app.state().selection.map(|(_, interval)| interval)
}

fn interval(x: [f64; 2], y: [f64; 2]) -> Option<Interval> {
    Some(Interval {
        x: Some(x),
        y: Some(y),
    })
}

/// A drag reports each next interval in the scales' domains, and settles the last one.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_draws_a_box() {
    let mut app = brushed(figure()).await;
    send(&mut app, drag([70.0, 70.0], [150.0, 130.0])).await;
    assert!(same(selection(&app), Some(BOX)));
    let state = app.state();
    assert_eq!(state.changes.len(), 1);
    assert_eq!(state.settles.len(), 1);
    assert_eq!(state.settles[0].0, 0);
    assert!(same(state.settles[0].1, Some(BOX)));
    assert_eq!(boxes(&app), vec![[70.0, 70.0, 80.0, 60.0]]);

    // A drag in either direction reports the same ranges, from low to high. With one selection
    // for the figure, a box in the other panel replaces the first.
    send(&mut app, drag([400.0, 130.0], [320.0, 70.0])).await;
    assert_eq!(app.state().selection.map(|(panel, _)| panel), Some(1));
    assert!(same(selection(&app), Some(BOX)));
    assert_eq!(boxes(&app), vec![[320.0, 70.0, 80.0, 60.0]]);
}

/// A drag inside a box moves it, and a drag on an edge or a corner moves those edges.
#[tokio::test(flavor = "current_thread")]
async fn drags_move_and_resize_a_box() {
    let mut app = brushed(figure()).await;
    send(&mut app, drag([70.0, 70.0], [150.0, 130.0])).await;

    send(&mut app, drag([110.0, 100.0], [130.0, 110.0])).await;
    assert!(same(selection(&app), interval([20.0, 60.0], [1.0, 7.0])));

    // The right edge is at x = 170 after the move. The y range keeps its value exactly.
    let y = selection(&app).unwrap().y;
    send(&mut app, drag([170.0, 100.0], [190.0, 100.0])).await;
    assert!(same(selection(&app), interval([20.0, 70.0], [1.0, 7.0])));
    assert_eq!(selection(&app).unwrap().y, y);

    // The top-left corner is at [90, 80].
    send(&mut app, drag([90.0, 80.0], [60.0, 60.0])).await;
    assert!(same(selection(&app), interval([5.0, 70.0], [1.0, 9.0])));

    // A move stops at the area's edge.
    send(&mut app, drag([150.0, 100.0], [400.0, 100.0])).await;
    assert!(same(selection(&app), interval([35.0, 100.0], [1.0, 9.0])));
}

/// Escape and a cancellation restore the value at the press and settle it, and the press's
/// release is no click.
#[tokio::test(flavor = "current_thread")]
async fn escape_and_cancellation_restore_the_value() {
    let brushes = Brushes::new(BRUSHES, 10).with(figure()).with(picks());
    let mut app = app(|builder| builder.tool(brushes)).await;
    send(&mut app, drag([70.0, 70.0], [150.0, 130.0])).await;
    let pressed = mouse(ElementState::Pressed);

    send(&mut app, [moved([110.0, 100.0]), pressed.clone()]).await;
    send(&mut app, [moved([130.0, 110.0])]).await;
    assert!(!same(selection(&app), Some(BOX)));
    send(&mut app, [escape()]).await;
    assert!(same(selection(&app), Some(BOX)));
    assert!(same(app.state().settles.last().unwrap().1, Some(BOX)));
    // Moves and the release after Escape change nothing.
    let settled = app.state().settles.len();
    send(
        &mut app,
        [moved([200.0, 140.0]), mouse(ElementState::Released)],
    )
    .await;
    assert!(same(selection(&app), Some(BOX)));
    assert_eq!(app.state().settles.len(), settled);

    // With one selection for the figure, Escape during a box in the other panel restores the box
    // in the panel where it was.
    send(&mut app, [moved([320.0, 70.0]), pressed.clone()]).await;
    send(&mut app, [moved([400.0, 130.0])]).await;
    assert_eq!(app.state().selection.map(|(panel, _)| panel), Some(1));
    send(&mut app, [escape(), mouse(ElementState::Released)]).await;
    assert_eq!(boxes(&app), vec![[70.0, 70.0, 80.0, 60.0]]);

    // The release of a press that Escape canceled picks no instance.
    let [moved_to_dot, _, released] = click(DOTS.1[0]);
    send(
        &mut app,
        [moved_to_dot, pressed.clone(), escape(), released],
    )
    .await;
    assert!(app.state().picked.is_empty());
    assert!(same(selection(&app), Some(BOX)));

    // A cancellation, such as focus loss, restores the value too.
    let settled = app.state().settles.len();
    send(&mut app, [moved([100.0, 90.0]), pressed]).await;
    send(&mut app, [moved([130.0, 110.0])]).await;
    send(&mut app, [WindowEvent::WindowFocused(false)]).await;
    assert!(same(selection(&app), Some(BOX)));
    assert_eq!(app.state().settles.len(), settled + 1);
}

/// A click clears the box in its area, and a click without a box creates none.
#[tokio::test(flavor = "current_thread")]
async fn clicks_clear_a_box() {
    let mut app = brushed(figure()).await;
    send(&mut app, drag([70.0, 70.0], [150.0, 130.0])).await;
    send(&mut app, click([200.0, 140.0])).await;
    assert_eq!(selection(&app), None);
    assert_eq!(app.state().settles.last(), Some(&(0, None)));
    assert!(boxes(&app).is_empty());

    let reports = app.state().changes.len();
    send(&mut app, click([200.0, 140.0])).await;
    // A drag that stays within the click distance is a click.
    send(&mut app, drag([100.0, 100.0], [103.0, 102.0])).await;
    assert_eq!(selection(&app), None);
    assert_eq!(app.state().changes.len(), reports);
}

/// An inactive brush lets presses through to a pan below, and an active brush claims them.
#[tokio::test(flavor = "current_thread")]
async fn an_active_brush_keeps_presses_from_panning() {
    let mut app = app(|builder| {
        builder
            .tool(pan_zoom())
            .tool(Brushes::new(BRUSHES, 10).with(figure()))
    })
    .await;
    let home = app.state().x.numeric_interval_domain().unwrap();

    app.state_mut().select = false;
    send(&mut app, drag([100.0, 100.0], [140.0, 100.0])).await;
    assert_ne!(app.state().x.numeric_interval_domain().unwrap(), home);
    assert_eq!(selection(&app), None);

    app.state_mut().x = Chart::default().x;
    app.state_mut().select = true;
    send(&mut app, drag([90.0, 80.0], [140.0, 120.0])).await;
    assert_eq!(app.state().x.numeric_interval_domain().unwrap(), home);
    assert!(selection(&app).is_some());

    // Wheel events and double clicks reach the pan below.
    send(&mut app, [wheel()]).await;
    assert_ne!(app.state().x.numeric_interval_domain().unwrap(), home);
    let at = [200.0, 140.0];
    send(&mut app, click(at)).await;
    send(&mut app, click(at)).await;
    assert_eq!(app.state().x.numeric_interval_domain().unwrap(), home);
}

/// With one selection for each panel, each keyed area keeps its own box.
#[tokio::test(flavor = "current_thread")]
async fn keyed_areas() {
    let mut app = brushed(per_panel()).await;
    send(&mut app, drag([70.0, 70.0], [150.0, 130.0])).await;
    send(&mut app, drag([320.0, 70.0], [400.0, 130.0])).await;
    assert!(same(app.state().boxes.get(&0).copied(), Some(BOX)));
    assert!(same(app.state().boxes.get(&1).copied(), Some(BOX)));
    assert_eq!(boxes(&app).len(), 2);

    // A removed panel's box no longer shows, and it shows again with its panel.
    app.state_mut().panels.pop();
    send(&mut app, [rebuild()]).await;
    assert_eq!(boxes(&app), vec![[70.0, 70.0, 80.0, 60.0]]);
    app.state_mut().panels.push((1, RIGHT));
    send(&mut app, [rebuild()]).await;
    assert_eq!(boxes(&app).len(), 2);
}

/// A zoom of the scales below draws the box again over the same data.
#[tokio::test(flavor = "current_thread")]
async fn a_zoom_keeps_the_box_on_its_data() {
    let mut app = app(|builder| {
        builder
            .tool(pan_zoom())
            .tool(Brushes::new(BRUSHES, 10).with(figure()))
    })
    .await;
    send(&mut app, drag([70.0, 70.0], [150.0, 130.0])).await;
    send(&mut app, [wheel()]).await;
    assert!(same(selection(&app), Some(BOX)));

    let state = app.state();
    let [left, right] = [10.0, 50.0].map(|x| LEFT.x + pixel(&state.x, x));
    let [top, bottom] = [8.0, 2.0].map(|y| LEFT.y + pixel(&state.y, y));
    let [drawn] = boxes(&app)[..] else {
        panic!("one box is drawn");
    };
    let expected = [left, top, right - left, bottom - top];
    assert!((drawn.iter().zip(expected)).all(|(a, b)| (a - b).abs() < 1e-3));
    assert_ne!(drawn, [70.0, 70.0, 80.0, 60.0]);
}

/// The cursor shows what a press would do.
#[tokio::test(flavor = "current_thread")]
async fn cursors() {
    let mut app = brushed(figure()).await;
    send(&mut app, drag([70.0, 70.0], [150.0, 130.0])).await;
    let cases = [
        ([110.0, 100.0], CursorIcon::Move),
        ([150.0, 100.0], CursorIcon::EwResize),
        ([110.0, 70.0], CursorIcon::NsResize),
        ([70.0, 70.0], CursorIcon::NwseResize),
        ([153.0, 128.0], CursorIcon::NwseResize),
        ([150.0, 70.0], CursorIcon::NeswResize),
        ([200.0, 140.0], CursorIcon::Crosshair),
        ([20.0, 20.0], CursorIcon::Default),
    ];
    for (at, cursor) in cases {
        send(&mut app, [moved(at)]).await;
        assert_eq!(app.cursor(), cursor, "at {at:?}");
    }

    // A gesture keeps its cursor wherever the pointer goes.
    send(
        &mut app,
        [
            moved([150.0, 100.0]),
            mouse(ElementState::Pressed),
            moved([20.0, 20.0]),
        ],
    )
    .await;
    assert_eq!(app.cursor(), CursorIcon::EwResize);
    send(&mut app, [mouse(ElementState::Released)]).await;

    app.state_mut().select = false;
    send(&mut app, [moved([110.0, 100.0])]).await;
    assert_eq!(app.cursor(), CursorIcon::Default);
}

/// A click selects an instance by the key in its own mark's rows, Shift+click toggles one, and a
/// click away from every instance clears the selection.
#[tokio::test(flavor = "current_thread")]
async fn clicks_select_instances_by_key() {
    let mut app = brushed_with(picks()).await;
    let [dot, middle_dot, _] = DOTS.1;
    let [ring, _] = RINGS.1;
    send(&mut app, click(middle_dot)).await;
    assert_eq!(app.state().picked, [11]);
    // The first ring is row 0 of its own rows.
    send(&mut app, shift_click(ring)).await;
    assert_eq!(app.state().picked, [11, 20]);
    send(&mut app, shift_click(middle_dot)).await;
    assert_eq!(app.state().picked, [20]);
    send(&mut app, click(dot)).await;
    assert_eq!(app.state().picked, [10]);
    send(&mut app, shift_click([430.0, 140.0])).await;
    assert_eq!(app.state().picked, [10]);
    send(&mut app, click([470.0, 120.0])).await;
    assert!(app.state().picked.is_empty());
}

/// A keyed point brush acts on the clicks in its areas, each on its own selection, so a click
/// away from every instance clears the selection of its area alone.
#[tokio::test(flavor = "current_thread")]
async fn keyed_clicks_select_in_their_areas() {
    let mut app = brushed_with(panel_picks()).await;
    let [_, middle_dot, _] = DOTS.1;
    let [ring, _] = RINGS.1;
    send(&mut app, click(middle_dot)).await;
    send(&mut app, shift_click(ring)).await;
    assert_eq!(app.state().panel_picks[&1], [11, 20]);
    // A click in the left panel and a click outside both panels leave the right panel's picks.
    send(&mut app, click([100.0, 100.0])).await;
    send(&mut app, click([275.0, 100.0])).await;
    assert_eq!(app.state().panel_picks[&1], [11, 20]);
    assert!(!app.state().panel_picks.contains_key(&0));
    send(&mut app, click([470.0, 120.0])).await;
    assert!(app.state().panel_picks[&1].is_empty());
}

/// A press whose pointer moves less than a click allows picks over a pan, which doesn't pan for
/// it, so the target tree stays current. A press that moves farther picks nothing, even when its
/// release lies over the instance under its press.
#[tokio::test(flavor = "current_thread")]
async fn clicks_pick_and_pans_do_not() {
    let mut app = picks_over_pan().await;
    let home = app.state().x.numeric_interval_domain().unwrap();
    let [dot, middle_dot, _] = DOTS.1;
    let (pressed, released) = (mouse(ElementState::Pressed), mouse(ElementState::Released));
    let jitter = moved([middle_dot[0] + 2.0, middle_dot[1] + 1.0]);
    send(
        &mut app,
        [moved(middle_dot), pressed.clone(), jitter, released.clone()],
    )
    .await;
    assert_eq!(app.state().picked, [11]);
    assert_eq!(app.state().x.numeric_interval_domain().unwrap(), home);

    let away = moved([dot[0] + 40.0, dot[1]]);
    send(&mut app, [moved(dot), pressed, away, moved(dot), released]).await;
    assert_eq!(app.state().picked, [11]);
}

/// A click selects nothing while the target scene's tree is stale, and selects again after the
/// tree builds.
#[tokio::test(flavor = "current_thread")]
async fn a_stale_target_tree_selects_nothing() {
    let mut app = picks_over_pan().await;
    let [_, dot, last] = DOTS.1;
    send(&mut app, [moved([420.0, 120.0]), wheel()]).await;
    send(&mut app, click(dot)).await;
    assert!(app.state().picked.is_empty());

    // The zoom settles and builds the tree.
    std::thread::sleep(Duration::from_millis(150));
    app.refresh().await;
    send(&mut app, click(last)).await;
    assert_eq!(app.state().picked, [12]);
}

/// A click that selects an instance inside a box keeps the box, and a click on empty space
/// clears both.
#[tokio::test(flavor = "current_thread")]
async fn a_click_on_an_instance_keeps_the_box() {
    let brushes = Brushes::new(BRUSHES, 10).with(figure()).with(picks());
    let mut app = app(|builder| builder.tool(brushes)).await;
    send(&mut app, drag([320.0, 70.0], [480.0, 140.0])).await;
    let [_, dot, _] = DOTS.1;
    send(&mut app, click(dot)).await;
    assert_eq!(app.state().picked, [11]);
    assert!(selection(&app).is_some());

    send(&mut app, click([430.0, 115.0])).await;
    assert!(app.state().picked.is_empty());
    assert_eq!(selection(&app), None);
}

/// The right panel's lasso.
fn right_lasso(app: &AvengerApp<Chart>) -> Lasso<i64> {
    app.state().lassos.get(&1).cloned().unwrap_or_default()
}

fn sorted(mut keys: Vec<i64>) -> Vec<i64> {
    keys.sort();
    keys
}

/// Around the first two dots, from `[330, 85]` to `[420, 115]`.
const AROUND_DOTS: [[f32; 2]; 4] = [[330.0, 85.0], [420.0, 85.0], [420.0, 115.0], [330.0, 115.0]];

/// A lasso selects the instances inside its path and reports the path in the scales' domains. A
/// Shift+drag adds to the lasso, and a click clears it.
#[tokio::test(flavor = "current_thread")]
async fn a_lasso_selects_the_instances_inside_it() {
    let mut app = brushed_with(lassos()).await;
    send(&mut app, trace(&AROUND_DOTS)).await;
    let lasso = right_lasso(&app);
    assert_eq!(sorted(lasso.keys.clone()), [10, 11]);
    let [polygon] = &lasso.polygons[..] else {
        panic!("the lasso has one polygon");
    };
    // The first vertex is at [330, 85], 30 pixels into the panel and 35 pixels down.
    let [x, y] = polygon[0];
    assert!((x - 15.0).abs() < 1e-6 && (y - 6.5).abs() < 1e-6);
    assert!(polygon.len() > 20);

    let mut add = vec![key(NamedKey::Shift, ElementState::Pressed)];
    add.extend(trace(&[
        [335.0, 120.0],
        [365.0, 120.0],
        [365.0, 140.0],
        [335.0, 140.0],
    ]));
    add.push(key(NamedKey::Shift, ElementState::Released));
    send(&mut app, add).await;
    let lasso = right_lasso(&app);
    assert_eq!(sorted(lasso.keys.clone()), [10, 11, 20]);
    assert_eq!(lasso.polygons.len(), 2);

    send(&mut app, trace(&AROUND_DOTS)).await;
    assert_eq!(right_lasso(&app).polygons.len(), 1);

    send(&mut app, click([470.0, 70.0])).await;
    assert!(right_lasso(&app).is_empty());
}

/// Each area of a keyed lasso can select among instances of its own.
#[tokio::test(flavor = "current_thread")]
async fn a_lasso_selects_among_the_instances_of_its_area() {
    let rings_on_the_right = lassos().instances_in(|_, panel| match panel {
        1 => Instances::marks([RINGS.0]).key(id),
        _ => Instances::marks([DOTS.0, RINGS.0]).key(id),
    });
    let mut app = brushed_with(rings_on_the_right).await;
    // Around the first two dots and both rings.
    let around = [[330.0, 85.0], [420.0, 85.0], [420.0, 140.0], [330.0, 140.0]];
    send(&mut app, trace(&around)).await;
    assert_eq!(sorted(right_lasso(&app).keys), [20, 21]);
}

/// A zoom of the scales below draws the lasso again over the same data.
#[tokio::test(flavor = "current_thread")]
async fn a_zoom_keeps_the_lasso_on_its_data() {
    let pan = PanZoom::xy(PLOT, |_| RIGHT, |s: &mut Chart| &mut s.x, |s| &mut s.y);
    let brushes = Brushes::new(BRUSHES, 10).with(lassos());
    let mut app = app(|builder| builder.tool(pan).tool(brushes)).await;
    send(&mut app, trace(&AROUND_DOTS)).await;
    send(&mut app, [moved([400.0, 120.0]), wheel()]).await;

    let state = app.state();
    // The lasso around the dots spans x from 15 to 60 and y from 3.5 to 6.5, and draws from the
    // panel's top-left corner.
    let xs = [pixel(&state.x, 15.0), pixel(&state.x, 60.0)];
    let ys = [pixel(&state.y, 6.5), pixel(&state.y, 3.5)];
    let expected = [xs[0], ys[0], xs[1], ys[1]];
    let bounds = lasso_bounds(&app);
    assert!((bounds.iter().zip(expected)).all(|(at, expected)| (at - expected).abs() < 1e-3));
    assert_ne!(bounds, [30.0, 35.0, 120.0, 65.0]);
}

/// The bounds of the right panel's lasso region, `[left, top, right, bottom]`, from the panel's
/// top-left corner.
fn lasso_bounds(app: &AvengerApp<Chart>) -> [f32; 4] {
    let path = path(app, "brush_lasso");
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for event in path.iter() {
        if let lyon_path::PathEvent::Begin { at } | lyon_path::PathEvent::Line { to: at, .. } =
            event
        {
            bounds = [
                bounds[0].min(at.x),
                bounds[1].min(at.y),
                bounds[2].max(at.x),
                bounds[3].max(at.y),
            ];
        }
    }
    bounds
}

/// A drag that reports a change settles even when it returns to the value at its press.
#[tokio::test(flavor = "current_thread")]
async fn a_drag_back_to_its_start_settles() {
    let mut app = brushed(figure()).await;
    send(&mut app, drag([70.0, 70.0], [150.0, 130.0])).await;
    let settles = app.state().settles.len();
    let there_and_back = [
        moved([110.0, 100.0]),
        mouse(ElementState::Pressed),
        moved([130.0, 100.0]),
        moved([110.0, 100.0]),
        mouse(ElementState::Released),
    ];
    send(&mut app, there_and_back).await;
    assert!(same(selection(&app), Some(BOX)));
    assert_eq!(app.state().settles.len(), settles + 1);
}

/// During a drag, a lasso draws its path closed and filled, and reports its selection as the
/// path grows, though each report leaves the plot's tree stale. Escape restores the lasso at the
/// press.
#[tokio::test(flavor = "current_thread")]
async fn a_lasso_during_its_drag() {
    let mut app = brushed_with(lassos()).await;
    // Around the first dot only, without a release.
    let around_first = [[330.0, 85.0], [370.0, 85.0], [370.0, 115.0], [330.0, 115.0]];
    let mut drawing = trace(&around_first);
    drawing.pop();
    send(&mut app, drawing).await;
    let closed = (path(&app, "brush_lasso_path").iter())
        .any(|event| matches!(event, lyon_path::PathEvent::End { close: true, .. }));
    assert!(closed);
    let fill = marks(&app, "brush_lasso_path").find_map(|(_, mark)| match mark {
        SceneMark::Path(path) => path.fill.first().cloned(),
        _ => None,
    });
    assert_ne!(fill, Some(ColorOrGradient::Color([0.0; 4])));
    assert_eq!(right_lasso(&app).keys, [10]);
    assert_eq!(right_lasso(&app).polygons.len(), 1);

    // The path grows on around the second dot.
    send(&mut app, [moved([420.0, 115.0]), moved([420.0, 85.0])]).await;
    assert_eq!(sorted(right_lasso(&app).keys), [10, 11]);

    send(&mut app, [escape(), mouse(ElementState::Released)]).await;
    assert!(right_lasso(&app).is_empty());
}

/// With a change interval, a drag reports at most once per interval while its box follows every
/// move. A pause reports the latest value, and the release settles it.
#[tokio::test(flavor = "current_thread")]
async fn a_change_interval_spaces_the_reports() {
    let brushes = Brushes::new(BRUSHES, 10)
        .with(figure())
        .with_change_interval(Duration::from_millis(50));
    let mut app = app(|builder| builder.tool(brushes)).await;
    send(
        &mut app,
        [moved([70.0, 70.0]), mouse(ElementState::Pressed)],
    )
    .await;
    // Moves 10 ms apart, and the wakes that the host would send between them.
    let moves = 20;
    for step in 1..=moves {
        let step = step as f32;
        send(&mut app, [moved([70.0 + 4.0 * step, 70.0 + 3.0 * step])]).await;
        app.refresh().await;
    }
    let reports = app.state().changes.len();
    assert!((1..moves).contains(&reports), "{reports} reports");
    // The box shows the last move, whether or not the app has its value yet.
    assert_eq!(boxes(&app), vec![[70.0, 70.0, 80.0, 60.0]]);

    std::thread::sleep(Duration::from_millis(60));
    app.refresh().await;
    assert!(same(selection(&app), Some(BOX)));
    send(&mut app, [mouse(ElementState::Released)]).await;
    assert_eq!(app.state().settles.len(), 1);
    assert!(same(app.state().settles[0].1, Some(BOX)));
}

/// The pieces of the path mark named `name` in the brushes' scene: one for a solid outline, and
/// one for each dash of a dashed outline.
fn pieces(app: &AvengerApp<Chart>, name: &str) -> usize {
    (path(app, name).iter())
        .filter(|event| matches!(event, lyon_path::PathEvent::Begin { .. }))
        .count()
}

/// A dash pattern draws the outlines of boxes and lassos in dashes.
#[tokio::test(flavor = "current_thread")]
async fn a_dash_pattern_dashes_the_outlines() {
    let dashed = BrushStyle {
        stroke_dash: Some(vec![4.0, 3.0]),
        ..Default::default()
    };
    let mut solid = brushed_with(figure()).await;
    send(&mut solid, drag([70.0, 70.0], [150.0, 130.0])).await;
    assert_eq!(pieces(&solid, "brush_outline"), 1);

    let brushes = Brushes::new(BRUSHES, 10)
        .with(figure())
        .with(lassos())
        .with_style(dashed);
    let mut app = app(|builder| builder.tool(brushes)).await;
    send(&mut app, drag([70.0, 70.0], [150.0, 130.0])).await;
    // The box's outline runs 280 pixels, in 7-pixel steps of dash and gap.
    assert_eq!(pieces(&app, "brush_outline"), 40);
    app.state_mut().select = false;
    send(&mut app, trace(&AROUND_DOTS)).await;
    assert!(pieces(&app, "brush_lasso_outline") > 20);
}

/// A Shift+drag around the lasso in the other direction fills the union of the two polygons, so
/// their overlap stays filled.
#[tokio::test(flavor = "current_thread")]
async fn added_lassos_fill_their_union() {
    let mut app = brushed_with(lassos()).await;
    send(&mut app, trace(&AROUND_DOTS)).await;
    let mut reversed = AROUND_DOTS;
    reversed.reverse();
    let mut add = vec![key(NamedKey::Shift, ElementState::Pressed)];
    add.extend(trace(&reversed));
    add.push(key(NamedKey::Shift, ElementState::Released));
    send(&mut app, add).await;
    assert_eq!(right_lasso(&app).polygons.len(), 2);
    let filled = avenger_scenegraph::render::fill::filled_contours(&path(&app, "brush_lasso"), 0.1);
    assert!(!filled.is_empty());
}

/// A switch from panning to selecting settles a zoom that hasn't settled, so a click right after
/// the switch finds the instance under it.
#[tokio::test(flavor = "current_thread")]
async fn a_switch_from_panning_settles_its_zoom() {
    let mut app = app(|builder| {
        builder
            .tool_when(|s| !s.select, pan_zoom())
            .tool(Brushes::new(BRUSHES, 10).with(picks()))
    })
    .await;
    app.state_mut().select = false;
    send(&mut app, [moved([100.0, 100.0]), wheel()]).await;
    app.state_mut().select = true;
    send(&mut app, click(DOTS.1[0])).await;
    assert_eq!(app.state().picked, [DOTS.2[0]]);
}

/// A brush without handles draws none on its box, even while the box changes.
#[tokio::test(flavor = "current_thread")]
async fn a_brush_without_handles() {
    for handles in [true, false] {
        let mut app = brushed(figure().handles(handles)).await;
        let press = mouse(ElementState::Pressed);
        send(
            &mut app,
            [moved([70.0, 70.0]), press, moved([150.0, 130.0])],
        )
        .await;
        assert_eq!(marks(&app, "brush_handles").next().is_some(), handles);
    }
}

/// A brush at an aspect keeps its boxes at that shape: a new box grows to reach the pointer and
/// shrinks to fit its area, a corner resizes the box from the opposite corner, and an edge moves
/// it.
#[tokio::test(flavor = "current_thread")]
async fn an_aspect_keeps_the_shape_of_boxes() {
    let drawn = |app: &AvengerApp<Chart>, expected: [f32; 4]| {
        let [found] = boxes(app)[..] else {
            panic!("the brush draws one box");
        };
        let close = (found.iter().zip(expected)).all(|(a, b)| (a - b).abs() < 1e-3);
        assert!(close, "{found:?} isn't {expected:?}");
    };
    // The left panel is 200 by 100 pixels, from [50, 50].
    let mut app = brushed(figure().aspect(Aspect::Area)).await;
    send(&mut app, drag([70.0, 70.0], [110.0, 130.0])).await;
    drawn(&app, [70.0, 70.0, 120.0, 60.0]);

    let mut app = brushed(figure().aspect(Aspect::Ratio(1.0))).await;
    send(&mut app, drag([200.0, 70.0], [240.0, 140.0])).await;
    drawn(&app, [200.0, 70.0, 50.0, 50.0]);
    send(&mut app, drag([70.0, 70.0], [110.0, 90.0])).await;
    drawn(&app, [70.0, 70.0, 40.0, 40.0]);
    send(&mut app, drag([110.0, 110.0], [130.0, 120.0])).await;
    drawn(&app, [70.0, 70.0, 60.0, 60.0]);
    send(&mut app, drag([130.0, 100.0], [140.0, 100.0])).await;
    drawn(&app, [80.0, 70.0, 60.0, 60.0]);
}

/// Instances from the app state apply at each click, so a change of the marks that the app draws
/// needs no new brush.
#[tokio::test(flavor = "current_thread")]
async fn instances_from_the_app_state_apply_at_each_click() {
    let brush = picks().instances_from(|s: &Chart| Instances::marks(s.pickable.clone()).key(id));
    let mut app = brushed_with(brush).await;
    let [_, middle_dot, _] = DOTS.1;
    let [ring, _] = RINGS.1;
    app.state_mut().pickable = vec![RINGS.0];
    send(&mut app, click(middle_dot)).await;
    assert!(app.state().picked.is_empty());
    send(&mut app, click(ring)).await;
    assert_eq!(app.state().picked, [20]);
    app.state_mut().pickable = vec![DOTS.0];
    send(&mut app, click(middle_dot)).await;
    assert_eq!(app.state().picked, [11]);
}
