//! The cursor is a view of the app's state: the app reads it after each update, whatever the
//! handlers asked for, and reports it when it changes.

use std::sync::Arc;

use async_trait::async_trait;
use avenger_app::app::{AvengerApp, BuiltScene, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_common::cursor::CursorStyle;
use avenger_common::time::Instant;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{WindowCursorMoved, WindowEvent};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

#[derive(Clone, Default)]
struct State {
    /// The pointer's x.
    x: f32,
}

struct Empty;

#[async_trait]
impl SceneGraphBuilder<State> for Empty {
    async fn build(&self, _: &mut State) -> Result<BuiltScene, AvengerAppError> {
        Ok(SceneGraph {
            marks: Vec::new(),
            width: 10.0,
            height: 10.0,
            origin: [0.0; 2],
        }
        .into())
    }
}

/// Records the pointer's x, and asks for nothing.
struct Track;

#[async_trait]
impl EventStreamHandler<State> for Track {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        if let SceneGraphEvent::CursorMoved(moved) = event {
            state.x = moved.position[0];
        }
        UpdateStatus::UNCHANGED
    }
}

/// A crosshair over the right half of the scene, and the default cursor elsewhere.
#[tokio::test(flavor = "current_thread")]
async fn the_cursor_follows_the_state() {
    let track = EventStreamConfig {
        types: vec![SceneGraphEventType::CursorMoved],
        ..Default::default()
    };
    let streams: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> =
        vec![(track, Arc::new(Track))];
    let mut app = AvengerApp::try_new(
        State::default(),
        Arc::new(Empty),
        streams,
        LabelEngine::new(Default::default()),
    )
    .await
    .unwrap()
    .with_cursor(|state: &State| {
        if state.x > 5.0 {
            CursorStyle::Crosshair
        } else {
            CursorStyle::Default
        }
    });
    assert_eq!(app.cursor(), CursorStyle::Default);
    let mut move_to = async |x| {
        let moved = WindowEvent::CursorMoved(WindowCursorMoved { position: [x, 1.0] });
        app.update(&moved, Instant::now()).await.unwrap().cursor
    };
    assert_eq!(move_to(2.0).await, None);
    assert_eq!(move_to(8.0).await, Some(CursorStyle::Crosshair));
    assert_eq!(move_to(9.0).await, None, "the same cursor is no change");
    assert_eq!(move_to(3.0).await, Some(CursorStyle::Default));
}
