//! Named scene requests, dependent tool updates, hit-testing tree freshness, and gated tools.

use std::sync::{Arc, Mutex};

use arrow_array::cast::AsArray;
use arrow_array::types::Int32Type;
use arrow_array::{ArrayRef, Int32Array, RecordBatch};
use async_trait::async_trait;
use avenger_app::app::{
    AppBuild, AvengerApp, BuiltScene, Scene, Scenes, Tool, ToolHandler, ToolInstall,
};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::Refresh;
use avenger_common::time::Instant;
use avenger_eventstream::manager::{EventStreamHandler, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{DebounceConfig, EventStreamConfig, Rebuild};
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
    WindowMouseInput, WindowResizeEvent,
};
use avenger_geometry::data::SceneData;
use avenger_geometry::query::{Query, QueryShape, Relation};
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::scene_graph::SceneGraph;

const CHART: &str = "chart";
const UNDER: &str = "under";
const TOP: &str = "top";

#[derive(Default)]
struct State {
    /// Number of chart builds.
    builds: usize,
    /// Whether the chart data has the wrong number of rows.
    bad_rows: bool,
    /// Whether the chart's build fails.
    fail: bool,
    /// Whether the chart's tree was current when the top tool's handler last heard a move.
    heard_chart_current: Option<bool>,
    /// The flag that gated tools read, which each key flips in a `Switch` app.
    on: bool,
    /// Whether a `Switch` app's build turns `on` on.
    on_at_build: bool,
    /// The press events that `Records` tools heard, by tool label.
    heard: Vec<(&'static str, &'static str)>,
    /// The x positions of the moves that `Records` tools heard.
    moves: Vec<f32>,
    /// What the top tool observed in its last update, which it records through this shared
    /// handle, since an update reads the app state but can't change it.
    top: Arc<Mutex<TopSeen>>,
}

/// What the top tool observed in its last update.
#[derive(Clone, Default)]
struct TopSeen {
    /// Update count copied from the private top state.
    updates: usize,
    /// Chart mark name and row build number.
    seen: Option<(String, Option<i32>)>,
    /// Chart tree freshness.
    chart_current: bool,
    /// Top tree freshness after the update.
    current: bool,
}

fn scene(width: f32) -> SceneGraph {
    SceneGraph {
        marks: Vec::new(),
        width,
        height: 10.0,
        origin: [0.0; 2],
    }
}

/// Key events that request a chart rebuild.
fn keys() -> EventStreamConfig {
    EventStreamConfig {
        types: vec![SceneGraphEventType::KeyPress],
        ..Default::default()
    }
}

/// Draws one mark named for its build number, with that number in its data row.
struct Chart;

#[async_trait]
impl AppBuild<State> for Chart {
    fn scenes(&self) -> Vec<Scene<State>> {
        vec![Scene::new(CHART, 0).stream(keys(), RedrawChart)]
    }

    async fn build(
        &self,
        state: &mut State,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(CHART) {
            return Ok(());
        }
        if state.fail {
            return Err(AvengerAppError::InternalError("chart".to_string()));
        }
        state.builds += 1;
        let name = format!("chart {}", state.builds);
        let rect = SceneRectMark {
            name: name.clone(),
            width: Some(20.0.into()),
            height: Some(10.0.into()),
            ..Default::default()
        };
        let rows = if state.bad_rows { 2 } else { 1 };
        let build: ArrayRef = Arc::new(Int32Array::from(vec![state.builds as i32; rows]));
        let mut data = SceneData::new();
        data.insert(
            name,
            Arc::new(RecordBatch::try_from_iter([("build", build)]).unwrap()),
        );
        let scene = SceneGraph {
            marks: vec![rect.into()],
            ..scene(20.0)
        };
        scenes.set(CHART, BuiltScene { scene, data })
    }
}

/// Requests a chart graph rebuild without rebuilding its tree.
struct RedrawChart;

#[async_trait]
impl EventStreamHandler<State> for RedrawChart {
    async fn handle(&self, _: &SceneGraphEvent, _: &mut State, _: &SceneTrees<'_>) -> Rebuild {
        Rebuild::scene(CHART)
    }
}

/// An overlay that updates when the pointer moves or a lower scene changes, and reads the
/// chart's tree and rows on each update. Its private state counts updates.
struct Top;

/// Notes whether the chart's tree is current, and requests a top scene rebuild.
struct Moves;

#[async_trait]
impl ToolHandler<State, usize> for Moves {
    async fn handle(
        &self,
        _: &SceneGraphEvent,
        state: &mut State,
        _: &mut usize,
        trees: &SceneTrees<'_>,
    ) -> Rebuild {
        state.heard_chart_current = Some(trees.below()[0].is_current());
        Rebuild::scene(TOP)
    }
}

#[async_trait]
impl Tool<State> for Top {
    type State = usize;

    fn install(&self, tool: &mut ToolInstall<'_, State, usize>) {
        let moves = EventStreamConfig {
            types: vec![SceneGraphEventType::CursorMoved],
            ..Default::default()
        };
        tool.scene(TOP, 10).stream(TOP, moves, Moves);
    }

    async fn update(
        &self,
        app: &State,
        own: &mut usize,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(TOP) && !scenes.below(TOP).any(|below| below.changed()) {
            return Ok(());
        }
        *own += 1;
        let mut top = app.top.lock().unwrap();
        top.updates = *own;
        top.chart_current = scenes.is_current(CHART);
        let chart = scenes.tree(CHART);
        let mark = Query::new(QueryShape::Point([5.0, 5.0]), Relation::Intersects).limit(1);
        top.seen = (chart.query(&mark).first()).map(|instance| {
            let datum = chart.datum(&instance.mark_instance);
            let build = datum.map(|datum| {
                let builds = datum.rows.column_by_name("build").unwrap();
                builds.as_primitive::<Int32Type>().value(datum.row)
            });
            (instance.mark_instance.name.clone(), build)
        });
        scenes.set(TOP, scene(20.0).into())?;
        top.current = scenes.is_current(TOP);
        Ok(())
    }
}

/// A tool below the top scene that rebuilds on a press.
struct Under;

/// Requests an under scene rebuild.
struct AskUnder;

#[async_trait]
impl ToolHandler<State, ()> for AskUnder {
    async fn handle(
        &self,
        _: &SceneGraphEvent,
        _: &mut State,
        _: &mut (),
        _: &SceneTrees<'_>,
    ) -> Rebuild {
        Rebuild::scene(UNDER)
    }
}

#[async_trait]
impl Tool<State> for Under {
    type State = ();

    fn install(&self, tool: &mut ToolInstall<'_, State, ()>) {
        let presses = EventStreamConfig {
            types: vec![SceneGraphEventType::MouseDown],
            ..Default::default()
        };
        tool.scene(UNDER, 5).stream(UNDER, presses, AskUnder);
    }

    async fn update(
        &self,
        _: &State,
        _: &mut (),
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(UNDER) {
            return Ok(());
        }
        scenes.set(UNDER, scene(20.0).into())
    }
}

fn engine() -> avenger_typst_label::LabelEngine {
    avenger_typst_label::LabelEngine::new(Default::default())
}

async fn app_with(state: State) -> Result<AvengerApp<State>, AvengerAppError> {
    AvengerApp::builder(state, Chart, engine())
        .tool(Top)
        .build()
        .await
}

async fn app() -> AvengerApp<State> {
    app_with(State::default()).await.unwrap()
}

fn moved() -> WindowEvent {
    moved_to([5.0, 5.0])
}

fn moved_to(position: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position })
}

fn press() -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state: ElementState::Pressed,
        button: MouseButton::Left,
    })
}

fn key() -> WindowEvent {
    WindowEvent::KeyboardInput(WindowKeyboardInput {
        key: Key::Character('a'),
        state: ElementState::Pressed,
        text: None,
    })
}

fn release() -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state: ElementState::Released,
        button: MouseButton::Left,
    })
}

fn resize(size: [f32; 2]) -> WindowEvent {
    WindowEvent::WindowResize(WindowResizeEvent { size })
}

/// Return what the top tool observed in its last update.
fn top(app: &AvengerApp<State>) -> TopSeen {
    app.state().top.lock().unwrap().clone()
}

/// Return the chart mark and row build number observed by the top tool.
fn seen(app: &AvengerApp<State>) -> Option<(String, Option<i32>)> {
    top(app).seen
}

/// The initial build includes all trees. Tools can read the tree and rows that the app's
/// build just set.
#[tokio::test(flavor = "current_thread")]
async fn the_first_build_sets_every_scene_with_its_tree() {
    let app = app().await;
    assert_eq!(app.scenes().len(), 2);
    assert_eq!(app.state().builds, 1);
    assert_eq!(seen(&app), Some(("chart 1".to_string(), Some(1))));
    assert!(top(&app).chart_current);
    assert_eq!(top(&app).updates, 1);
}

/// A pointer move updates only the top. A key updates the chart and its dependent top.
#[tokio::test(flavor = "current_thread")]
async fn a_request_sets_its_scene_and_what_reads_it() {
    let mut app = app().await;
    let now = Instant::now();
    assert_eq!(app.update(&moved(), now).await.scenes, [1]);
    assert_eq!(app.state().builds, 1);
    assert_eq!(top(&app).updates, 2);
    assert_eq!(app.update(&key(), now).await.scenes, [0, 1]);
    assert_eq!(app.state().builds, 2);
}

/// A graph-only request preserves the old tree and rows until a full refresh.
/// A dependent scene set without its own request gets a current tree.
#[tokio::test(flavor = "current_thread")]
async fn a_scene_request_leaves_the_tree_stale() {
    let mut app = app().await;
    let now = Instant::now();
    // A key carries the pointer's position, so it needs a move first.
    app.update(&moved(), now).await;
    assert!(!top(&app).current);
    app.update(&key(), now).await;
    assert_eq!(seen(&app), Some(("chart 1".to_string(), Some(1))));
    assert!(!top(&app).chart_current);
    assert!(top(&app).current);
    app.waker().request(Refresh::Rebuild);
    assert_eq!(app.refresh().await.scenes, [0, 1]);
    assert_eq!(seen(&app), Some(("chart 3".to_string(), Some(3))));
    assert!(top(&app).chart_current);
}

/// A handler sees whether the trees below its scene are current: a graph-only rebuild leaves the
/// chart's tree stale until a rebuild with its tree.
#[tokio::test(flavor = "current_thread")]
async fn handlers_see_whether_the_trees_below_are_current() {
    let mut app = app().await;
    let now = Instant::now();
    app.update(&moved(), now).await;
    assert_eq!(app.state().heard_chart_current, Some(true));
    app.update(&key(), now).await;
    app.update(&moved(), now).await;
    assert_eq!(app.state().heard_chart_current, Some(false));
    app.waker().request(Refresh::Rebuild);
    app.refresh().await;
    app.update(&moved(), now).await;
    assert_eq!(app.state().heard_chart_current, Some(true));
}

/// A lower tool updates before the top even when registered later.
#[tokio::test(flavor = "current_thread")]
async fn tools_update_from_the_lowest_scenes_up() {
    let builder = AvengerApp::builder(State::default(), Chart, engine());
    let mut app = builder.tool(Top).tool(Under).build().await.unwrap();
    let now = Instant::now();
    app.update(&moved(), now).await;
    let updates = top(&app).updates;
    assert_eq!(app.update(&press(), now).await.scenes, [1, 2]);
    assert_eq!(top(&app).updates, updates + 1);
}

/// Nonzero resizes rebuild all scenes and trees. Zero-size resizes request no scenes.
#[tokio::test(flavor = "current_thread")]
async fn a_resize_requests_every_scene_unless_the_window_has_no_size() {
    let mut app = app().await;
    let now = Instant::now();
    assert_eq!(
        app.update(&resize([300.0, 200.0]), now).await.scenes,
        [0, 1]
    );
    assert_eq!(app.size(), [300.0, 200.0]);
    assert!(app
        .update(&resize([0.0, 200.0]), now)
        .await
        .scenes
        .is_empty());
    assert_eq!(app.size(), [300.0, 200.0]);
}

/// Mismatched row counts produce a tree with working hits but no data rows.
#[tokio::test(flavor = "current_thread")]
async fn rows_that_miscount_their_marks_leave_the_tree_without_rows() {
    let app = app_with(State {
        bad_rows: true,
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(seen(&app), Some(("chart 1".to_string(), None)));
}

/// A failed build preserves scenes it did not replace. An initial build error
/// fails app construction.
#[tokio::test(flavor = "current_thread")]
async fn a_failed_build_keeps_its_scenes() {
    let mut app = app().await;
    let chart = app.scenes()[0].clone();
    app.state_mut().fail = true;
    let now = Instant::now();
    app.update(&moved(), now).await;
    assert!(app.update(&key(), now).await.scenes.is_empty());
    assert!(Arc::ptr_eq(&app.scenes()[0], &chart));
    let failing = State {
        fail: true,
        ..Default::default()
    };
    assert!(app_with(failing).await.is_err());
}

/// Declares scenes without setting their graphs.
struct Declares(Vec<(&'static str, i32)>);

#[async_trait]
impl AppBuild<State> for Declares {
    fn scenes(&self) -> Vec<Scene<State>> {
        (self.0.iter())
            .map(|(name, z)| Scene::new(name, *z))
            .collect()
    }

    async fn build(&self, _: &mut State, _: &mut Scenes<'_>) -> Result<(), AvengerAppError> {
        Ok(())
    }
}

/// Attaches a stream to the top's scene without declaring it.
struct Stray;

impl Tool<State> for Stray {
    type State = usize;

    fn install(&self, tool: &mut ToolInstall<'_, State, usize>) {
        tool.stream(TOP, keys(), Moves);
    }
}

/// The builder rejects missing scenes, duplicate names or z-orders, and streams
/// attached to undeclared scenes.
#[tokio::test(flavor = "current_thread")]
async fn the_builder_rejects_scenes_that_do_not_fit() {
    let cases = [
        vec![],
        vec![(CHART, 0), (CHART, 1)],
        vec![(CHART, 0), (TOP, 0)],
    ];
    for scenes in cases {
        let app = AvengerApp::builder(State::default(), Declares(scenes), engine()).build();
        assert!(app.await.is_err());
    }
    let stray = AvengerApp::builder(State::default(), Chart, engine()).tool(Stray);
    assert!(stray.build().await.is_err());
}

/// Draws an empty chart, and flips the `on` flag on each key without requesting a scene. Its build
/// turns `on` on when `on_at_build` is set.
struct Switch;

/// Flips the `on` flag.
struct Flip;

#[async_trait]
impl EventStreamHandler<State> for Flip {
    async fn handle(&self, _: &SceneGraphEvent, state: &mut State, _: &SceneTrees<'_>) -> Rebuild {
        state.on = !state.on;
        Rebuild::none()
    }
}

#[async_trait]
impl AppBuild<State> for Switch {
    fn scenes(&self) -> Vec<Scene<State>> {
        vec![Scene::new(CHART, 0).stream(keys(), Flip)]
    }

    async fn build(
        &self,
        state: &mut State,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(CHART) {
            return Ok(());
        }
        state.on |= state.on_at_build;
        scenes.set(CHART, scene(20.0).into())
    }
}

/// A gated tool hears nothing and draws nothing while it is off. Turning on draws its scene
/// without a request from the handler that changed the flag, and turning off clears it. The
/// tool's own state lasts while it is off.
#[tokio::test(flavor = "current_thread")]
async fn a_gated_tool_turns_on_and_off_with_the_state() {
    let builder = AvengerApp::builder(State::default(), Switch, engine());
    let mut app = builder.tool_when(|s| s.on, Top).build().await.unwrap();
    let now = Instant::now();
    assert_eq!(top(&app).updates, 0);
    assert!(app.update(&moved(), now).await.scenes.is_empty());
    assert_eq!(app.state().heard_chart_current, None);
    assert_eq!(app.update(&key(), now).await.scenes, [1]);
    assert_eq!(top(&app).updates, 1);
    assert_eq!(app.scenes()[1].width, 20.0);
    app.update(&moved(), now).await;
    assert_eq!(app.state().heard_chart_current, Some(true));
    assert_eq!(top(&app).updates, 2);
    assert_eq!(app.update(&key(), now).await.scenes, [1]);
    assert_eq!(app.scenes()[1].width, app.size()[0]);
    app.update(&key(), now).await;
    assert_eq!(top(&app).updates, 3);
}

/// Records the presses that its tool's stream hears, with its label, and the x position of each
/// move.
struct Heard(&'static str);

#[async_trait]
impl ToolHandler<State, ()> for Heard {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        _: &mut (),
        _: &SceneTrees<'_>,
    ) -> Rebuild {
        let kind = match event {
            SceneGraphEvent::MouseDown(_) => "down",
            SceneGraphEvent::MouseUp(_) => "up",
            SceneGraphEvent::MouseCancel(_) => "cancel",
            SceneGraphEvent::CursorMoved(moved) => {
                state.moves.push(moved.position[0]);
                return Rebuild::none();
            }
            _ => "other",
        };
        state.heard.push((self.0, kind));
        Rebuild::none()
    }
}

/// A tool with one stream on the chart's scene, which hears `config`'s events and records them
/// with `label`.
struct Records {
    label: &'static str,
    config: EventStreamConfig,
}

impl Tool<State> for Records {
    type State = ();

    fn install(&self, tool: &mut ToolInstall<'_, State, ()>) {
        tool.stream(CHART, self.config.clone(), Heard(self.label));
    }
}

/// A tool that records the presses on the chart's scene.
fn presses(label: &'static str) -> Records {
    let types = [
        SceneGraphEventType::MouseDown,
        SceneGraphEventType::MouseUp,
        SceneGraphEventType::MouseCancel,
    ];
    let config = EventStreamConfig {
        types: types.to_vec(),
        ..Default::default()
    };
    Records { label, config }
}

/// A gated tool that turns off during a press hears a cancellation, and no other tool does.
#[tokio::test(flavor = "current_thread")]
async fn a_tool_that_turns_off_during_a_press_hears_a_cancellation() {
    let state = State {
        on: true,
        ..Default::default()
    };
    let builder = AvengerApp::builder(state, Switch, engine());
    let builder = builder.tool_when(|s| s.on, presses("gated"));
    let mut app = builder.tool(presses("always")).build().await.unwrap();
    let now = Instant::now();
    for event in [moved(), press(), key(), release()] {
        app.update(&event, now).await;
    }
    let heard = [
        ("gated", "down"),
        ("always", "down"),
        ("gated", "cancel"),
        ("always", "up"),
    ];
    assert_eq!(app.state().heard, heard);
}

/// A gated tool turns on when the app's build changes the state that its predicate reads.
#[tokio::test(flavor = "current_thread")]
async fn a_build_can_turn_a_tool_on() {
    let state = State {
        on_at_build: true,
        ..Default::default()
    };
    let builder = AvengerApp::builder(state, Switch, engine());
    let app = builder.tool_when(|s| s.on, Top).build().await.unwrap();
    assert!(app.state().on);
    assert_eq!(top(&app).updates, 1);
}

/// A tool that turns off handles a move that hasn't settled at once, so that it finishes its
/// work, and the move never reaches the tool again after it turns back on.
#[tokio::test(flavor = "current_thread")]
async fn a_tool_that_turns_off_handles_its_pending_events_at_once() {
    let state = State {
        on: true,
        ..Default::default()
    };
    let builder = AvengerApp::builder(state, Switch, engine());
    let moves = EventStreamConfig {
        types: vec![SceneGraphEventType::CursorMoved],
        debounce: Some(DebounceConfig::new(50)),
        ..Default::default()
    };
    let settles = Records {
        label: "settles",
        config: moves,
    };
    let mut app = builder.tool_when(|s| s.on, settles).build().await.unwrap();
    app.update(&moved_to([15.0, 5.0]), Instant::now()).await;
    assert!(app.state().moves.is_empty());
    app.update(&key(), Instant::now()).await;
    assert_eq!(app.state().moves, [15.0]);
    app.update(&moved_to([10.0, 5.0]), Instant::now()).await;
    app.update(&key(), Instant::now()).await;
    std::thread::sleep(std::time::Duration::from_millis(80));
    app.refresh().await;
    assert_eq!(app.state().moves, [15.0]);
}
