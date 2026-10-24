//! Mouse events target the topmost mark drawn under the pointer, exactly, so a mark that only
//! touches the pointer's mark doesn't take its events.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_common::time::Instant;
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, Rebuild};
use avenger_eventstream::window::{WindowCursorMoved, WindowEvent};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use futures::executor::block_on;

/// The instances that a handler saw the pointer enter.
type Entered = Arc<Mutex<Vec<Option<usize>>>>;

struct Record(Entered);

#[async_trait]
impl EventStreamHandler<()> for Record {
    async fn handle(&self, event: &SceneGraphEvent, _: &mut (), _: &SceneTrees<'_>) -> Rebuild {
        let instance = event.mark_instance().and_then(|mark| mark.instance_index);
        self.0.lock().unwrap().push(instance);
        Rebuild::none()
    }
}

/// Two bars edge to edge, the upper one drawn last.
fn stacked_bars() -> [SceneGraphRTree; 1] {
    let bars = SceneRectMark {
        len: 2,
        x: 0.0.into(),
        y: vec![0.0, 10.0].into(),
        width: Some(10.0.into()),
        height: Some(10.0.into()),
        ..Default::default()
    };
    let scene = SceneGraph {
        marks: vec![bars.into()],
        width: 20.0,
        height: 20.0,
        origin: [0.0; 2],
    };
    [SceneGraphRTree::from_scene_graph(
        &scene,
        &avenger_typst_label::LabelEngine::new(Default::default()),
    )]
}

/// The pointer a twentieth of a pixel inside the lower bar enters it, though the upper bar is
/// drawn over it nearby, and enters the upper bar once it crosses their edge.
#[test]
fn the_pointer_enters_the_bar_drawn_under_it() {
    block_on(async {
        let entered = Entered::default();
        let mut manager = EventStreamManager::new();
        let config = EventStreamConfig {
            types: vec![SceneGraphEventType::MarkMouseEnter],
            ..Default::default()
        };
        manager.register_handler(0, config, Arc::new(Record(entered.clone())));
        let (rtree, now) = (stacked_bars(), Instant::now());
        for y in [9.95, 10.05] {
            let moved = WindowEvent::CursorMoved(WindowCursorMoved { position: [5.0, y] });
            manager.dispatch_event(&moved, &mut (), &rtree, now).await;
        }
        assert_eq!(*entered.lock().unwrap(), [Some(0), Some(1)]);
    });
}
