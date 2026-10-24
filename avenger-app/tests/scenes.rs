//! An app draws named scenes. Handlers request scenes by name, and each build runs the app's
//! build and then each tool's update, which set the requested scenes and leave the rest, with
//! their graphs, trees, and rows, as they were.

use std::sync::Arc;

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
use avenger_eventstream::stream::{EventStreamConfig, Rebuild};
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
    /// The chart's builds.
    builds: usize,
    /// The name of the mark in the chart's tree, and the build in its row, as the top's last
    /// update saw them.
    seen: Option<(String, Option<i32>)>,
    /// Whether the chart's tree showed its latest graph, as the top's last update saw it.
    chart_current: bool,
    /// Whether the chart's rows miscount its mark's instances.
    bad_rows: bool,
    /// Whether the chart's build fails.
    fail: bool,
    /// The top's updates, which its own state counts.
    top_updates: usize,
    /// Whether the top's tree showed its latest graph after the top's last update.
    top_current: bool,
}

fn scene(width: f32) -> SceneGraph {
    SceneGraph {
        marks: Vec::new(),
        width,
        height: 10.0,
        origin: [0.0; 2],
    }
}

/// Key presses, which ask for the chart's scene.
fn keys() -> EventStreamConfig {
    EventStreamConfig {
        types: vec![SceneGraphEventType::KeyPress],
        ..Default::default()
    }
}

/// Draws the chart: one mark named for its build, whose row holds the build.
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

/// Asks for the chart's scene, without its tree.
struct RedrawChart;

#[async_trait]
impl EventStreamHandler<State> for RedrawChart {
    async fn handle(&self, _: &SceneGraphEvent, _: &mut State, _: &SceneTrees<'_>) -> Rebuild {
        Rebuild::scene(CHART)
    }
}

/// A tool over the chart, which follows the pointer, updates when a scene below it changes, and
/// reads the chart's tree and rows. Its state counts its updates.
struct Top;

/// Asks for the top's scene.
struct Moves;

#[async_trait]
impl ToolHandler<State, usize> for Moves {
    async fn handle(
        &self,
        _: &SceneGraphEvent,
        _: &mut State,
        _: &mut usize,
        _: &SceneTrees<'_>,
    ) -> Rebuild {
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
        app: &mut State,
        own: &mut usize,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(TOP) && !scenes.below(TOP).any(|below| below.changed()) {
            return Ok(());
        }
        *own += 1;
        app.top_updates = *own;
        app.chart_current = scenes.is_current(CHART);
        let chart = scenes.tree(CHART);
        let mark = Query::new(QueryShape::Point([5.0, 5.0]), Relation::Intersects).limit(1);
        app.seen = (chart.query(&mark).first()).map(|instance| {
            let datum = chart.datum(&instance.mark_instance);
            let build = datum.map(|datum| {
                let builds = datum.rows.column_by_name("build").unwrap();
                builds.as_primitive::<Int32Type>().value(datum.row)
            });
            (instance.mark_instance.name.clone(), build)
        });
        scenes.set(TOP, scene(20.0).into())?;
        app.top_current = scenes.is_current(TOP);
        Ok(())
    }
}

/// A tool with a scene under the top, which a press asks for.
struct Under;

/// Asks for the scene under the top.
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
        _: &mut State,
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
    WindowEvent::CursorMoved(WindowCursorMoved {
        position: [5.0, 5.0],
    })
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
    })
}

fn resize(size: [f32; 2]) -> WindowEvent {
    WindowEvent::WindowResize(WindowResizeEvent { size })
}

/// The mark in the chart's tree, and the build in its row, as the top last saw them.
fn seen(app: &AvengerApp<State>) -> Option<(&str, Option<i32>)> {
    let seen = app.state().seen.as_ref();
    seen.map(|(name, build)| (name.as_str(), *build))
}

/// The first build requests every scene with its tree, and a tool reads the tree that the app's
/// build set in the same build, with its rows.
#[tokio::test(flavor = "current_thread")]
async fn the_first_build_sets_every_scene_with_its_tree() {
    let app = app().await;
    assert_eq!(app.scenes().len(), 2);
    assert_eq!(app.state().builds, 1);
    assert_eq!(seen(&app), Some(("chart 1", Some(1))));
    assert!(app.state().chart_current);
    assert_eq!(app.state().top_updates, 1);
}

/// A request sets only the scenes that it names, and the scenes that read them: a move sets the
/// top alone, and a key sets the chart and the top that reads it.
#[tokio::test(flavor = "current_thread")]
async fn a_request_sets_its_scene_and_what_reads_it() {
    let mut app = app().await;
    let now = Instant::now();
    assert_eq!(app.update(&moved(), now).await.scenes, [1]);
    assert_eq!(app.state().builds, 1);
    assert_eq!(app.state().top_updates, 2);
    assert_eq!(app.update(&key(), now).await.scenes, [0, 1]);
    assert_eq!(app.state().builds, 2);
}

/// A request for a scene without its tree keeps the tree, with the rows of the build that made
/// it, and the tree is stale until a refresh rebuilds every scene with its tree. Only the scene
/// that the request names goes stale: the top, which its update sets because the chart below it
/// changed, gets its tree.
#[tokio::test(flavor = "current_thread")]
async fn a_scene_request_leaves_the_tree_stale() {
    let mut app = app().await;
    let now = Instant::now();
    // A key carries the pointer's position, so it needs a move first.
    app.update(&moved(), now).await;
    assert!(!app.state().top_current);
    app.update(&key(), now).await;
    assert_eq!(seen(&app), Some(("chart 1", Some(1))));
    assert!(!app.state().chart_current);
    assert!(app.state().top_current);
    app.waker().request(Refresh::Rebuild);
    assert_eq!(app.refresh().await.scenes, [0, 1]);
    assert_eq!(seen(&app), Some(("chart 3", Some(3))));
    assert!(app.state().chart_current);
}

/// Tools update from the lowest of their scenes up, so the top sees the scene under it change,
/// though the app added the top first.
#[tokio::test(flavor = "current_thread")]
async fn tools_update_from_the_lowest_scenes_up() {
    let builder = AvengerApp::builder(State::default(), Chart, engine());
    let mut app = builder.tool(Top).tool(Under).build().await.unwrap();
    let now = Instant::now();
    app.update(&moved(), now).await;
    let updates = app.state().top_updates;
    assert_eq!(app.update(&press(), now).await.scenes, [1, 2]);
    assert_eq!(app.state().top_updates, updates + 1);
}

/// A resize to a nonzero size requests every scene with its tree, and a resize to a zero size,
/// as of a minimized window, requests none.
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

/// Rows that miscount their mark's instances leave the tree without rows, and the tree still
/// takes hits.
#[tokio::test(flavor = "current_thread")]
async fn rows_that_miscount_their_marks_leave_the_tree_without_rows() {
    let app = app_with(State {
        bad_rows: true,
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(seen(&app), Some(("chart 1", None)));
}

/// A build that fails keeps the scenes that it didn't set, and a first build that fails fails the
/// app.
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

/// Declares its scenes, and draws nothing.
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

/// The builder rejects an app without scenes, scenes that share a name or a z-order, and a tool's
/// stream on a scene that no one declared.
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
