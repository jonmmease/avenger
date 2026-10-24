//! The cursor and copy text that the app and its tools call for.

use async_trait::async_trait;
use avenger_app::app::{AppBuild, AvengerApp, Changes, Scene, Scenes, Tool, ToolInstall};
use avenger_app::error::AvengerAppError;
use avenger_common::cursor::CursorStyle;
use avenger_common::time::Instant;
use avenger_eventstream::window::{WindowCursorMoved, WindowEvent};
use avenger_scenegraph::scene_graph::SceneGraph;

/// What the app and each tool call for.
#[derive(Default)]
struct Calls {
    app: Option<CursorStyle>,
    /// The cursor and copy text of the tool with a scene, and of the tool without one.
    high: (Option<CursorStyle>, Option<String>),
    low: (Option<CursorStyle>, Option<String>),
}

fn empty(scenes: &Scenes<'_>) -> SceneGraph {
    let [width, height] = scenes.size();
    SceneGraph {
        marks: Vec::new(),
        width,
        height,
        origin: [0.0; 2],
    }
}

struct Chart;

#[async_trait]
impl AppBuild<Calls> for Chart {
    fn scenes(&self) -> Vec<Scene<Calls>> {
        vec![Scene::new("chart", 0)]
    }

    async fn build(&self, _: &mut Calls, scenes: &mut Scenes<'_>) -> Result<(), AvengerAppError> {
        if scenes.requested("chart") {
            scenes.set("chart", empty(scenes).into())?;
        }
        Ok(())
    }

    fn cursor(&self, calls: &Calls) -> Option<CursorStyle> {
        calls.app
    }
}

/// A tool that calls for what the app's state holds for it: with a scene at z-order 10 when
/// `high`, and else with none.
struct Asks {
    high: bool,
}

impl Asks {
    fn calls<'a>(&self, calls: &'a Calls) -> &'a (Option<CursorStyle>, Option<String>) {
        if self.high {
            &calls.high
        } else {
            &calls.low
        }
    }
}

#[async_trait]
impl Tool<Calls> for Asks {
    type State = ();

    fn install(&self, tool: &mut ToolInstall<'_, Calls, ()>) {
        if self.high {
            tool.scene("high", 10);
        }
    }

    async fn update(
        &self,
        _: &mut Calls,
        _: &mut (),
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if self.high && scenes.requested("high") {
            scenes.set("high", empty(scenes).into())?;
        }
        Ok(())
    }

    fn cursor(&self, calls: &Calls, _: &()) -> Option<CursorStyle> {
        self.calls(calls).0
    }

    fn copy_text(&self, calls: &Calls, _: &()) -> Option<String> {
        self.calls(calls).1.clone()
    }
}

/// Create an app whose tool with a scene registers before the tool without one.
async fn app() -> AvengerApp<Calls> {
    let engine = avenger_typst_label::LabelEngine::new(Default::default());
    (AvengerApp::builder(Calls::default(), Chart, engine))
        .tool(Asks { high: true })
        .tool(Asks { high: false })
        .build()
        .await
        .unwrap()
}

/// Change what the app and its tools call for, and return what an update reports.
async fn ask(app: &mut AvengerApp<Calls>, change: impl FnOnce(&mut Calls)) -> Changes {
    change(app.state_mut());
    let moved = WindowEvent::CursorMoved(WindowCursorMoved {
        position: [1.0, 1.0],
    });
    app.update(&moved, Instant::now()).await
}

/// The highest tool that calls for a cursor wins, where a tool without scenes lies below a tool
/// with one, then the app's cursor, and else the default. Each cursor is reported once.
#[tokio::test(flavor = "current_thread")]
async fn the_topmost_cursor_wins_and_is_reported_once() {
    let mut app = app().await;
    assert_eq!(app.cursor(), CursorStyle::Default);
    let changes = ask(&mut app, |calls| calls.app = Some(CursorStyle::Grab)).await;
    assert_eq!(changes.cursor, Some(CursorStyle::Grab));
    let changes = ask(&mut app, |calls| calls.low.0 = Some(CursorStyle::Text)).await;
    assert_eq!(changes.cursor, Some(CursorStyle::Text));
    let changes = ask(&mut app, |calls| calls.high.0 = Some(CursorStyle::Grabbing)).await;
    assert_eq!(changes.cursor, Some(CursorStyle::Grabbing));
    assert_eq!(ask(&mut app, |_| {}).await.cursor, None);
    let changes = ask(&mut app, |calls| *calls = Calls::default()).await;
    assert_eq!(changes.cursor, Some(CursorStyle::Default));
}

/// The highest tool that offers text gives the copy text, which is reported when it changes,
/// and empty text means none.
#[tokio::test(flavor = "current_thread")]
async fn the_topmost_copy_text_is_reported_when_it_changes() {
    let mut app = app().await;
    let text = |text: &str| Some(text.to_string());
    let changes = ask(&mut app, |calls| calls.low.1 = text("low")).await;
    assert_eq!(changes.copy_text, text("low"));
    let changes = ask(&mut app, |calls| calls.high.1 = text("high")).await;
    assert_eq!((changes.copy_text, app.copy_text()), (text("high"), "high"));
    assert_eq!(ask(&mut app, |_| {}).await.copy_text, None);
    let changes = ask(&mut app, |calls| *calls = Calls::default()).await;
    assert_eq!(changes.copy_text, text(""));
}
