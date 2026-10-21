//! A hit-testing tree whose rows don't match its scene takes the scene's hits without rows, so
//! hits never find the instances where an older scene drew them.

use std::sync::{Arc, Mutex};

use arrow_array::{ArrayRef, Float64Array, RecordBatch};
use async_trait::async_trait;
use avenger_app::app::{AvengerApp, BuiltScene, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::Refresh;
use avenger_common::time::Instant;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{WindowCursorMoved, WindowEvent};
use avenger_geometry::data::SceneData;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

#[derive(Clone, Default)]
struct State {
    moved: bool,
}

/// A point at x = 10 with its row, or once `moved`, at x = 90 with two rows for its one
/// instance, which don't match it.
struct Point;

#[async_trait]
impl SceneGraphBuilder<State> for Point {
    async fn build(&self, state: &mut State) -> Result<BuiltScene, AvengerAppError> {
        let (x, rows) = if state.moved { (90.0, 2) } else { (10.0, 1) };
        let point = SceneSymbolMark {
            name: "point".into(),
            len: 1,
            x: x.into(),
            y: 50.0.into(),
            size: 100.0.into(),
            ..Default::default()
        };
        let values: ArrayRef = Arc::new(Float64Array::from(vec![1.0; rows]));
        let mut data = SceneData::new();
        data.insert(
            "point",
            Arc::new(RecordBatch::try_from_iter([("v", values)]).unwrap()),
        );
        let scene = SceneGraph {
            marks: vec![point.into()],
            width: 100.0,
            height: 100.0,
            origin: [0.0; 2],
        };
        Ok(BuiltScene { scene, data })
    }
}

/// What a handler finds in the tree: whether hits at x = 10 and at x = 90 find the point, and
/// whether the tree holds the point's rows.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Found {
    at_10: bool,
    at_90: bool,
    rows: bool,
}

struct Look(Arc<Mutex<Found>>);

#[async_trait]
impl EventStreamHandler<State> for Look {
    async fn handle(
        &self,
        _: &SceneGraphEvent,
        _: &mut State,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let hit = |x| rtree.pick_top_mark_at_point(&[x, 50.0]).is_some();
        *self.0.lock().unwrap() = Found {
            at_10: hit(10.0),
            at_90: hit(90.0),
            rows: rtree.data().rows("point").is_some(),
        };
        UpdateStatus::UNCHANGED
    }
}

/// When a rebuild's rows don't match its scene, the tree takes the new scene's hits, without
/// rows, rather than keep the old scene's, and the rebuild fails.
#[tokio::test(flavor = "current_thread")]
async fn a_tree_whose_rows_dont_match_takes_the_scenes_hits_without_rows() {
    let found = Arc::new(Mutex::new(Found::default()));
    let moves = EventStreamConfig {
        types: vec![SceneGraphEventType::CursorMoved],
        ..Default::default()
    };
    let streams: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> =
        vec![(moves, Arc::new(Look(found.clone())))];
    let mut app = AvengerApp::try_new(
        State::default(),
        Arc::new(Point),
        streams,
        LabelEngine::new(Default::default()),
    )
    .await
    .unwrap();
    let look = async |app: &mut AvengerApp<State>, x: f32| {
        let moved = WindowEvent::CursorMoved(WindowCursorMoved {
            position: [x, 20.0],
        });
        app.update(&moved, Instant::now()).await.unwrap();
        *found.lock().unwrap()
    };
    let before = Found {
        at_10: true,
        at_90: false,
        rows: true,
    };
    assert_eq!(look(&mut app, 50.0).await, before);

    app.app_state_mut().moved = true;
    app.waker().request(Refresh::Rebuild);
    assert!(app.refresh().await.is_err());
    let after = Found {
        at_10: false,
        at_90: true,
        rows: false,
    };
    assert_eq!(look(&mut app, 51.0).await, after);
}
