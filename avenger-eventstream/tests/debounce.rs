//! Debounced streams handle a burst's last event once it settles, when the manager's caller
//! dispatches the wakes that are due.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_common::time::{Duration, Instant};
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{DebounceConfig, EventStreamConfig, Rebuild};
use avenger_eventstream::window::{WindowCursorMoved, WindowEvent};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;
use futures::executor::block_on;

/// The x positions of the moves that a handler saw.
type Seen = Arc<Mutex<Vec<f32>>>;

/// Records the moves that it sees.
struct Record(Seen);

#[async_trait]
impl EventStreamHandler<()> for Record {
    async fn handle(&self, event: &SceneGraphEvent, _: &mut (), _: &SceneTrees<'_>) -> Rebuild {
        if let SceneGraphEvent::CursorMoved(event) = event {
            self.0.lock().unwrap().push(event.position[0]);
        }
        Rebuild::none()
    }
}

fn record(seen: &Seen) -> Arc<Record> {
    Arc::new(Record(seen.clone()))
}

fn on(types: SceneGraphEventType) -> EventStreamConfig {
    EventStreamConfig {
        types: vec![types],
        ..Default::default()
    }
}

fn debounced(debounce: DebounceConfig) -> EventStreamConfig {
    EventStreamConfig {
        debounce: Some(debounce),
        ..on(SceneGraphEventType::CursorMoved)
    }
}

fn rtree() -> [SceneGraphRTree; 1] {
    let scene = SceneGraph {
        marks: Vec::new(),
        width: 10.0,
        height: 10.0,
        origin: [0.0; 2],
    };
    [SceneGraphRTree::from_scene_graph(
        &scene,
        &avenger_typst_label::LabelEngine::new(Default::default()),
    )]
}

fn moved(x: f32) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position: [x, 0.0] })
}

fn seen(seen: &Seen) -> Vec<f32> {
    seen.lock().unwrap().clone()
}

/// A burst of moves reaches a debounced handler once, as its last move, when `wait` passes after
/// it.
#[test]
fn a_debounced_burst_ends_with_its_last_event() {
    block_on(async {
        let moves = Seen::default();
        let mut manager = EventStreamManager::new();
        manager.register_handler(0, debounced(DebounceConfig::new(20)), record(&moves));
        let (rtree, start) = (rtree(), Instant::now());
        let at = |ms| start + Duration::from_millis(ms);
        for (x, ms) in [(1.0, 0), (2.0, 10), (3.0, 25)] {
            manager
                .dispatch_event(&moved(x), &mut (), &rtree, at(ms))
                .await;
        }
        assert!(seen(&moves).is_empty());
        assert_eq!(manager.next_wake(), Some(at(45)));
        manager.dispatch_due_wakes(&mut (), &rtree, at(44)).await;
        assert!(seen(&moves).is_empty());
        manager.dispatch_due_wakes(&mut (), &rtree, at(45)).await;
        assert_eq!(seen(&moves), [3.0]);
        assert_eq!(manager.next_wake(), None);
    });
}

/// With `leading`, a burst's first event reaches the handler at once and its last when the
/// burst settles, which `max_wait` limits. A burst of one event reaches the handler once.
#[test]
fn leading_bursts_start_at_once_and_max_wait_ends_them() {
    block_on(async {
        let moves = Seen::default();
        let mut manager = EventStreamManager::new();
        let config = DebounceConfig {
            wait: 20,
            max_wait: Some(30),
            leading: true,
        };
        manager.register_handler(0, debounced(config), record(&moves));
        let (rtree, start) = (rtree(), Instant::now());
        let at = |ms| start + Duration::from_millis(ms);
        for (x, ms) in [(1.0, 0), (2.0, 10), (3.0, 20)] {
            manager
                .dispatch_event(&moved(x), &mut (), &rtree, at(ms))
                .await;
        }
        assert_eq!(seen(&moves), [1.0]);
        assert_eq!(manager.next_wake(), Some(at(30)));
        manager.dispatch_due_wakes(&mut (), &rtree, at(30)).await;
        assert_eq!(seen(&moves), [1.0, 3.0]);

        manager
            .dispatch_event(&moved(4.0), &mut (), &rtree, at(100))
            .await;
        manager.dispatch_due_wakes(&mut (), &rtree, at(120)).await;
        assert_eq!(seen(&moves), [1.0, 3.0, 4.0]);
    });
}

/// A debounced stream that consumes takes each event it matches from the streams after it,
/// though its handler sees the event later.
#[test]
fn a_debounced_stream_consumes_each_event_it_takes() {
    block_on(async {
        let (first, second) = (Seen::default(), Seen::default());
        let mut manager = EventStreamManager::new();
        let consuming = EventStreamConfig {
            consume: true,
            ..debounced(DebounceConfig::new(20))
        };
        manager.register_handler(0, consuming, record(&first));
        manager.register_handler(0, on(SceneGraphEventType::CursorMoved), record(&second));
        let (rtree, start) = (rtree(), Instant::now());
        manager
            .dispatch_event(&moved(1.0), &mut (), &rtree, start)
            .await;
        manager
            .dispatch_due_wakes(&mut (), &rtree, start + Duration::from_millis(20))
            .await;
        assert_eq!(seen(&first), [1.0]);
        assert!(seen(&second).is_empty());
    });
}
