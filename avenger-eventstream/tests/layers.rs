//! Streams hear each event in layers, higher layers first, and within a layer in the order they
//! were added, so a stream that consumes an event takes it from the streams beneath it.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_common::time::Instant;
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, EventStreamFilter, UpdateStatus};
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;
use futures::executor::block_on;

/// Records its name for each press that it hears.
struct Record(&'static str, Arc<Mutex<Vec<&'static str>>>);

#[async_trait]
impl EventStreamHandler<()> for Record {
    async fn handle(&self, _: &SceneGraphEvent, _: &mut (), _: &SceneGraphRTree) -> UpdateStatus {
        self.1.lock().unwrap().push(self.0);
        UpdateStatus::UNCHANGED
    }
}

#[test]
fn higher_layers_hear_first_and_consume_for_those_beneath() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let presses = || EventStreamConfig {
        types: vec![SceneGraphEventType::MouseDown],
        ..Default::default()
    };
    // The top layer takes the presses right of x = 50.
    let right = EventStreamFilter(Arc::new(|event: &SceneGraphEvent| {
        event.position().is_some_and(|[x, _]| x > 50.0)
    }));
    let takes_right = EventStreamConfig {
        filter: Some(vec![right]),
        consume: true,
        ..presses()
    };
    let mut manager = EventStreamManager::new(());
    let record = |name| Arc::new(Record(name, heard.clone()));
    manager.register_handler(presses(), record("scene"));
    manager.register_handler_in_layer(2, takes_right, record("top"));
    manager.register_handler_in_layer(1, presses(), record("middle"));
    manager.register_handler(presses(), record("scene, added later"));
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
    let mut press = |x: f32| {
        heard.lock().unwrap().clear();
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
            block_on(manager.dispatch_event(&event, &rtree, Instant::now()));
        }
        heard.lock().unwrap().clone()
    };
    assert_eq!(press(10.0), ["middle", "scene", "scene, added later"]);
    assert_eq!(press(60.0), ["top"]);
}
