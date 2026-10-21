//! Overlays draw over the scene in the order the app takes them, and an overlay's own event
//! streams build the overlays alone: never the scene or its hit-testing tree.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use avenger_app::app::BuiltScene;
use avenger_app::app::{AvengerApp, OverlayBuilder, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::Refresh;
use avenger_common::time::Instant;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{WindowCursorMoved, WindowEvent};
use avenger_geometry::data::SceneData;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::{mark::SceneMark, rule::SceneRuleMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

#[derive(Clone, Default)]
struct State {
    builds: usize,
}

struct Empty;

#[async_trait]
impl SceneGraphBuilder<State> for Empty {
    async fn build(&self, state: &mut State) -> Result<BuiltScene, AvengerAppError> {
        state.builds += 1;
        Ok(SceneGraph {
            marks: Vec::new(),
            width: 10.0,
            height: 10.0,
            origin: [0.0; 2],
        }
        .into())
    }
}

/// An overlay of one mark named `name` for each move that its stream has seen.
struct Moves {
    name: &'static str,
    moves: Arc<AtomicUsize>,
}

impl OverlayBuilder<State> for Moves {
    fn build(
        &self,
        _: &State,
        _: &SceneGraph,
        _: &SceneData,
        _: &LabelEngine,
    ) -> Result<Vec<SceneMark>, AvengerAppError> {
        let mark = SceneRuleMark {
            name: self.name.to_string(),
            ..Default::default()
        };
        Ok(vec![mark.into(); self.moves.load(Ordering::SeqCst)])
    }

    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
        count(&self.moves)
    }
}

/// An overlay that fails to build once its stream has seen a move.
struct Fails(Arc<AtomicUsize>);

impl OverlayBuilder<State> for Fails {
    fn build(
        &self,
        _: &State,
        _: &SceneGraph,
        _: &SceneData,
        _: &LabelEngine,
    ) -> Result<Vec<SceneMark>, AvengerAppError> {
        match self.0.load(Ordering::SeqCst) {
            0 => Ok(Vec::new()),
            _ => Err(AvengerAppError::InternalError("a failing overlay".into())),
        }
    }

    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
        count(&self.0)
    }
}

/// A stream that counts moves in `moves`.
fn count(moves: &Arc<AtomicUsize>) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
    let config = EventStreamConfig {
        types: vec![SceneGraphEventType::CursorMoved],
        ..Default::default()
    };
    vec![(config, Arc::new(Count(moves.clone())))]
}

/// Counts moves, and asks for the overlays.
struct Count(Arc<AtomicUsize>);

#[async_trait]
impl EventStreamHandler<State> for Count {
    async fn handle(
        &self,
        _: &SceneGraphEvent,
        _: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        self.0.fetch_add(1, Ordering::SeqCst);
        UpdateStatus::OVERLAY
    }
}

/// An overlay's streams build the overlays and not the scene, and the overlays draw in the
/// order the app took them.
#[tokio::test(flavor = "current_thread")]
async fn overlays_draw_in_order_and_build_alone() {
    let overlay = |name| Moves {
        name,
        moves: Arc::default(),
    };
    let mut app = AvengerApp::try_new(
        State::default(),
        Arc::new(Empty),
        Vec::new(),
        LabelEngine::new(Default::default()),
    )
    .await
    .unwrap()
    .with_overlay(Arc::new(overlay("first")))
    .unwrap()
    .with_overlay(Arc::new(overlay("second")))
    .unwrap();
    let builds = app.app_state_mut().builds;
    let moved = WindowEvent::CursorMoved(WindowCursorMoved {
        position: [1.0, 1.0],
    });
    let changes = app.update(&moved, Instant::now()).await.unwrap();
    assert!(changes.scene.is_none());
    let overlay = changes.overlay.unwrap();
    let names: Vec<&str> = overlay.marks.iter().map(|mark| mark.mark_name()).collect();
    assert_eq!(names, ["first", "second"]);
    assert_eq!(app.app_state_mut().builds, builds);
}

/// An overlay that fails to build draws nothing, and the other overlays and the scene still
/// draw.
#[tokio::test(flavor = "current_thread")]
async fn a_failing_overlay_draws_nothing() {
    let mut app = AvengerApp::try_new(
        State::default(),
        Arc::new(Empty),
        Vec::new(),
        LabelEngine::new(Default::default()),
    )
    .await
    .unwrap()
    .with_overlay(Arc::new(Fails(Arc::default())))
    .unwrap()
    .with_overlay(Arc::new(Moves {
        name: "second",
        moves: Arc::default(),
    }))
    .unwrap();
    let moved = WindowEvent::CursorMoved(WindowCursorMoved {
        position: [1.0, 1.0],
    });
    let changes = app.update(&moved, Instant::now()).await.unwrap();
    let overlay = changes.overlay.unwrap();
    let names: Vec<&str> = overlay.marks.iter().map(|mark| mark.mark_name()).collect();
    assert_eq!(names, ["second"]);

    app.waker().request(Refresh::Rebuild);
    let changes = app.refresh().await.unwrap();
    assert!(changes.scene.is_some());
    assert_eq!(changes.overlay.unwrap().marks.len(), 1);
}
