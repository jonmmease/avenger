//! A press counts the presses in a row that it ends, as the DOM's `detail` counts them, within
//! the double click's time and distance.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_common::time::{Duration, Instant};
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;
use futures::executor::block_on;

/// Records the counts of the presses that it sees.
struct Record(Arc<Mutex<Vec<u32>>>);

#[async_trait]
impl EventStreamHandler<()> for Record {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        _: &mut (),
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        if let SceneGraphEvent::MouseDown(down) = event {
            self.0.lock().unwrap().push(down.count);
        }
        UpdateStatus::UNCHANGED
    }
}

#[test]
fn presses_in_a_row_count_on() {
    let counts = Arc::new(Mutex::new(Vec::new()));
    let mut manager = EventStreamManager::new(());
    let config = EventStreamConfig {
        types: vec![SceneGraphEventType::MouseDown],
        ..Default::default()
    };
    manager.register_handler(config, Arc::new(Record(counts.clone())));
    let scene = SceneGraph {
        marks: Vec::new(),
        width: 100.0,
        height: 100.0,
        origin: [0.0; 2],
    };
    let rtree = SceneGraphRTree::from_scene_graph(
        &scene,
        &avenger_typst_label::LabelEngine::new(Default::default()),
    );
    let start = Instant::now();
    let mut click = |x: f32, ms: u64| {
        let at = start + Duration::from_millis(ms);
        let moved = WindowEvent::CursorMoved(WindowCursorMoved {
            position: [x, 10.0],
        });
        let button = |state| {
            WindowEvent::MouseInput(WindowMouseInput {
                state,
                button: MouseButton::Left,
            })
        };
        for event in [
            moved,
            button(ElementState::Pressed),
            button(ElementState::Released),
        ] {
            block_on(manager.dispatch_event(&event, &rtree, at));
        }
    };
    click(10.0, 0);
    click(10.0, 100);
    click(11.0, 200);
    // Too far from the last press, and then too late after it.
    click(30.0, 300);
    click(30.0, 1_000);
    assert_eq!(*counts.lock().unwrap(), [1, 2, 3, 1, 1]);
}
