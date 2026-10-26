//! Scene event routing: z-order, consumption, hit targets, hover, and press ownership.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_common::time::Instant;
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{Disposition, EventStreamConfig, Rebuild};
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
    WindowMouseInput,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use futures::executor::block_on;

/// What the streams heard: the stream's name, the event's type, and its mark's name.
type Heard = Arc<Mutex<Vec<(&'static str, SceneGraphEventType, Option<String>)>>>;

/// Records events and the scene tree index, then requests a rebuild of its named scene.
struct Record(&'static str, usize, Heard);

#[async_trait]
impl<S: Send + Sync + 'static> EventStreamHandler<S> for Record {
    async fn handle(&self, event: &SceneGraphEvent, _: &mut S, trees: &SceneTrees<'_>) -> Rebuild {
        assert_eq!(
            trees.scene(),
            self.1,
            "{} hears in another scene's view",
            self.0
        );
        let mark = event.mark_instance().map(|mark| mark.name.clone());
        (self.2.lock().unwrap()).push((self.0, event.event_type(), mark));
        Rebuild::scene(self.0)
    }
}

/// Records events as [`Record`] does, and consumes them.
struct Consuming(Record);

#[async_trait]
impl<S: Send + Sync + 'static> EventStreamHandler<S> for Consuming {
    fn disposition(&self, _: &SceneGraphEvent, _: &S, _: &SceneTrees<'_>) -> Disposition {
        Disposition::Consume
    }

    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut S,
        trees: &SceneTrees<'_>,
    ) -> Rebuild {
        self.0.handle(event, state, trees).await
    }
}

fn on(types: &[SceneGraphEventType]) -> EventStreamConfig {
    EventStreamConfig {
        types: types.to_vec(),
        ..Default::default()
    }
}

/// Build a tree with one rectangle named `name`, bounded by `min` and `max`.
fn scene(name: &str, min: [f32; 2], max: [f32; 2]) -> SceneGraphRTree {
    let rect = SceneRectMark {
        name: name.to_string(),
        x: min[0].into(),
        y: min[1].into(),
        width: Some((max[0] - min[0]).into()),
        height: Some((max[1] - min[1]).into()),
        ..Default::default()
    };
    let scene = SceneGraph {
        marks: vec![rect.into()],
        width: 100.0,
        height: 100.0,
        origin: [0.0; 2],
    };
    let engine = avenger_typst_label::LabelEngine::new(Default::default());
    SceneGraphRTree::from_scene_graph(&scene, &engine)
}

/// A chart spanning (0, 0) to (20, 20), below a box spanning (0, 0) to (10, 10).
fn trees() -> Vec<SceneGraphRTree> {
    vec![
        scene("chart", [0.0, 0.0], [20.0, 20.0]),
        scene("box", [0.0, 0.0], [10.0, 10.0]),
    ]
}

fn moved(x: f32, y: f32) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position: [x, y] })
}

fn mouse(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

fn heard(heard: &Heard) -> Vec<(&'static str, SceneGraphEventType, Option<String>)> {
    std::mem::take(&mut *heard.lock().unwrap())
}

/// Create a manager with chart and box streams for `types`.
fn manager(types: &[SceneGraphEventType], heard: &Heard) -> EventStreamManager<()> {
    let mut manager = EventStreamManager::new();
    manager.register_handler(0, on(types), Arc::new(Record("chart", 0, heard.clone())));
    manager.register_handler(1, on(types), Arc::new(Record("box", 1, heard.clone())));
    manager
}

/// The top scene hears first, and each scene sees the mark under the pointer in its own tree.
#[test]
fn scenes_hear_from_the_top_with_their_own_marks() {
    block_on(async {
        let log = Heard::default();
        let mut manager = manager(&[SceneGraphEventType::CursorMoved], &log);
        let trees = trees();
        manager
            .dispatch_event(&moved(15.0, 15.0), &mut (), &trees, Instant::now())
            .await;
        let moved = SceneGraphEventType::CursorMoved;
        assert_eq!(
            heard(&log),
            [
                ("box", moved.clone(), None),
                ("chart", moved, Some("chart".to_string()))
            ]
        );
    });
}

/// Interactive hits stop pointer delivery after their scene receives the event.
/// Key events reach all scenes, including during a press.
#[test]
fn a_scenes_marks_stop_the_pointer_and_keys_fall_through() {
    block_on(async {
        let log = Heard::default();
        let types = [
            SceneGraphEventType::MouseDown,
            SceneGraphEventType::KeyPress,
        ];
        let mut manager = manager(&types, &log);
        let trees = trees();
        let now = Instant::now();
        manager
            .dispatch_event(&moved(5.0, 5.0), &mut (), &trees, now)
            .await;
        let rebuild = manager
            .dispatch_event(&mouse(ElementState::Pressed), &mut (), &trees, now)
            .await;
        assert_eq!(
            heard(&log),
            [(
                "box",
                SceneGraphEventType::MouseDown,
                Some("box".to_string())
            )]
        );
        assert_eq!(rebuild, Rebuild::scene("box"));
        let key = WindowEvent::KeyboardInput(WindowKeyboardInput {
            key: Key::Character('a'),
            state: ElementState::Pressed,
        });
        manager.dispatch_event(&key, &mut (), &trees, now).await;
        let names: Vec<_> = (heard(&log).into_iter()).map(|(name, ..)| name).collect();
        assert_eq!(names, ["box", "chart"]);
    });
}

/// Key events reach every scene before the pointer first enters the window, without marks.
#[test]
fn keys_reach_the_scenes_before_the_pointer_enters() {
    block_on(async {
        let log = Heard::default();
        let mut manager = manager(&[SceneGraphEventType::KeyPress], &log);
        let key = WindowEvent::KeyboardInput(WindowKeyboardInput {
            key: Key::Character('a'),
            state: ElementState::Pressed,
        });
        manager
            .dispatch_event(&key, &mut (), &trees(), Instant::now())
            .await;
        let press = SceneGraphEventType::KeyPress;
        assert_eq!(
            heard(&log),
            [("box", press.clone(), None), ("chart", press, None)]
        );
    });
}

/// A drag started on the chart continues across the box. A drag started on the box
/// never reaches the chart.
#[test]
fn a_press_keeps_its_drag_with_its_scenes() {
    block_on(async {
        let log = Heard::default();
        let types = [
            SceneGraphEventType::CursorMoved,
            SceneGraphEventType::MouseUp,
        ];
        let mut manager = manager(&types, &log);
        let trees = trees();
        let now = Instant::now();
        let chart_hears = |log: &Heard| heard(log).iter().any(|(name, ..)| *name == "chart");

        manager
            .dispatch_event(&moved(15.0, 15.0), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&mouse(ElementState::Pressed), &mut (), &trees, now)
            .await;
        heard(&log);
        manager
            .dispatch_event(&moved(5.0, 5.0), &mut (), &trees, now)
            .await;
        assert!(chart_hears(&log));
        manager
            .dispatch_event(&mouse(ElementState::Released), &mut (), &trees, now)
            .await;
        assert!(chart_hears(&log));
        // Without a press, the box takes the move.
        manager
            .dispatch_event(&moved(5.0, 5.0), &mut (), &trees, now)
            .await;
        assert!(!chart_hears(&log));

        manager
            .dispatch_event(&mouse(ElementState::Pressed), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&moved(15.0, 15.0), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&mouse(ElementState::Released), &mut (), &trees, now)
            .await;
        assert!(!chart_hears(&log));
    });
}

/// A release off the mark that its press lay on is no click, so the click that follows it is no
/// double click.
#[test]
fn a_release_off_its_mark_arms_no_double_click() {
    block_on(async {
        let log = Heard::default();
        let types = [SceneGraphEventType::Click, SceneGraphEventType::DoubleClick];
        let mut manager = manager(&types, &log);
        let trees = trees();
        let now = Instant::now();
        manager
            .dispatch_event(&moved(5.0, 5.0), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&mouse(ElementState::Pressed), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&moved(50.0, 50.0), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&mouse(ElementState::Released), &mut (), &trees, now)
            .await;
        assert_eq!(heard(&log), []);
        manager
            .dispatch_event(&mouse(ElementState::Pressed), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&mouse(ElementState::Released), &mut (), &trees, now)
            .await;
        let click = SceneGraphEventType::Click;
        assert_eq!(
            heard(&log),
            [("box", click.clone(), None), ("chart", click, None)]
        );
    });
}

/// Focus loss sends `MouseCancel` to the press owner and higher scenes.
/// Later pointer moves resume normal routing.
#[test]
fn focus_loss_ends_a_press() {
    block_on(async {
        let log = Heard::default();
        let types = [
            SceneGraphEventType::CursorMoved,
            SceneGraphEventType::MouseCancel,
        ];
        let mut manager = manager(&types, &log);
        let trees = trees();
        let now = Instant::now();
        manager
            .dispatch_event(&moved(5.0, 5.0), &mut (), &trees, now)
            .await;
        heard(&log);
        manager
            .dispatch_event(&mouse(ElementState::Pressed), &mut (), &trees, now)
            .await;
        for focused in [false, true] {
            manager
                .dispatch_event(&WindowEvent::WindowFocused(focused), &mut (), &trees, now)
                .await;
        }
        assert_eq!(
            heard(&log),
            [(
                "box",
                SceneGraphEventType::MouseCancel,
                Some("box".to_string())
            )]
        );
        manager
            .dispatch_event(&moved(15.0, 15.0), &mut (), &trees, now)
            .await;
        let names: Vec<_> = (heard(&log).into_iter()).map(|(name, ..)| name).collect();
        assert_eq!(names, ["box", "chart"]);
    });
}

/// Consumption in a higher scene cannot prevent the press owner from receiving
/// moves and release, so its drag can finish.
#[test]
fn scenes_above_a_press_cannot_keep_it_from_its_owner() {
    block_on(async {
        let log = Heard::default();
        let types = [
            SceneGraphEventType::CursorMoved,
            SceneGraphEventType::MouseUp,
        ];
        let mut manager = EventStreamManager::new();
        manager.register_handler(0, on(&types), Arc::new(Record("chart", 0, log.clone())));
        let consuming = Consuming(Record("box", 1, log.clone()));
        manager.register_handler(1, on(&types), Arc::new(consuming));
        let trees = trees();
        let now = Instant::now();
        manager
            .dispatch_event(&moved(15.0, 15.0), &mut (), &trees, now)
            .await;
        let names: Vec<_> = (heard(&log).into_iter()).map(|(name, ..)| name).collect();
        assert_eq!(names, ["box"]);
        manager
            .dispatch_event(&mouse(ElementState::Pressed), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&moved(5.0, 5.0), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&mouse(ElementState::Released), &mut (), &trees, now)
            .await;
        let (moved, up) = (
            SceneGraphEventType::CursorMoved,
            SceneGraphEventType::MouseUp,
        );
        let heard: Vec<_> = (heard(&log).into_iter())
            .map(|(name, event, _)| (name, event))
            .collect();
        assert_eq!(
            heard,
            [
                ("box", moved.clone()),
                ("chart", moved),
                ("box", up.clone()),
                ("chart", up)
            ]
        );
    });
}

/// Hover events use each scene's own marks. An interactive hit in a higher scene
/// clears hover targets in lower scenes.
#[test]
fn marks_enter_and_leave_in_their_own_scene() {
    block_on(async {
        let log = Heard::default();
        let types = [
            SceneGraphEventType::MarkMouseEnter,
            SceneGraphEventType::MarkMouseLeave,
        ];
        let mut manager = manager(&types, &log);
        let trees = trees();
        let now = Instant::now();
        let (enter, leave) = (
            SceneGraphEventType::MarkMouseEnter,
            SceneGraphEventType::MarkMouseLeave,
        );
        let mark = |name: &str| Some(name.to_string());
        manager
            .dispatch_event(&moved(15.0, 15.0), &mut (), &trees, now)
            .await;
        assert_eq!(heard(&log), [("chart", enter.clone(), mark("chart"))]);
        manager
            .dispatch_event(&moved(5.0, 5.0), &mut (), &trees, now)
            .await;
        assert_eq!(
            heard(&log),
            [
                ("box", enter.clone(), mark("box")),
                ("chart", leave.clone(), mark("chart"))
            ]
        );
        manager
            .dispatch_event(&WindowEvent::CursorLeft, &mut (), &trees, now)
            .await;
        assert_eq!(heard(&log), [("box", leave, mark("box"))]);
    });
}

/// Records the events that it handles, and consumes those that come while the state is true.
struct Claim(Heard);

#[async_trait]
impl EventStreamHandler<bool> for Claim {
    fn disposition(&self, _: &SceneGraphEvent, claims: &bool, _: &SceneTrees<'_>) -> Disposition {
        match claims {
            true => Disposition::Consume,
            false => Disposition::Handle,
        }
    }

    async fn handle(&self, event: &SceneGraphEvent, _: &mut bool, _: &SceneTrees<'_>) -> Rebuild {
        (self.0.lock().unwrap()).push(("claim", event.event_type(), None));
        Rebuild::none()
    }
}

/// A handler decides for each event whether its stream consumes it, so the presses that it only
/// handles still reach the scene below.
#[test]
fn a_handler_decides_which_events_its_stream_consumes() {
    block_on(async {
        let log = Heard::default();
        let down = [SceneGraphEventType::MouseDown];
        let mut manager = EventStreamManager::new();
        manager.register_handler(0, on(&down), Arc::new(Record("chart", 0, log.clone())));
        manager.register_handler(1, on(&down), Arc::new(Claim(log.clone())));
        let trees = trees();
        for mut claims in [false, true] {
            for event in [
                moved(15.0, 15.0),
                mouse(ElementState::Pressed),
                mouse(ElementState::Released),
            ] {
                (manager.dispatch_event(&event, &mut claims, &trees, Instant::now())).await;
            }
        }
        let down = SceneGraphEventType::MouseDown;
        assert_eq!(
            heard(&log),
            [
                ("claim", down.clone(), None),
                ("chart", down.clone(), Some("chart".to_string())),
                ("claim", down, None)
            ]
        );
    });
}

/// Records the count of each press.
struct Counts(Arc<Mutex<Vec<u32>>>);

#[async_trait]
impl EventStreamHandler<()> for Counts {
    async fn handle(&self, event: &SceneGraphEvent, _: &mut (), _: &SceneTrees<'_>) -> Rebuild {
        if let SceneGraphEvent::MouseDown(down) = event {
            self.0.lock().unwrap().push(down.count);
        }
        Rebuild::none()
    }
}

/// Consecutive presses increment the count within the double-click time and distance
/// thresholds. Exceeding either threshold resets it.
#[test]
fn presses_in_a_row_count_on() {
    let counts = Arc::new(Mutex::new(Vec::new()));
    let mut manager = EventStreamManager::new();
    let down = on(&[SceneGraphEventType::MouseDown]);
    manager.register_handler(0, down, Arc::new(Counts(counts.clone())));
    let trees = trees();
    let start = Instant::now();
    let mut click = |x: f32, ms: u64| {
        let at = start + std::time::Duration::from_millis(ms);
        for event in [
            moved(x, 15.0),
            mouse(ElementState::Pressed),
            mouse(ElementState::Released),
        ] {
            block_on(manager.dispatch_event(&event, &mut (), &trees, at));
        }
    };
    click(15.0, 0);
    click(15.0, 100);
    click(16.0, 200);
    // Too far from the last press, and then too late after it.
    click(30.0, 300);
    click(30.0, 1_000);
    assert_eq!(*counts.lock().unwrap(), [1, 2, 3, 1, 1]);
}

/// The left click that ends the second press in a row is a double click, and the next press
/// clicks again.
#[test]
fn the_second_press_in_a_row_double_clicks() {
    block_on(async {
        let log = Heard::default();
        let types = [SceneGraphEventType::Click, SceneGraphEventType::DoubleClick];
        let mut manager = manager(&types, &log);
        let trees = trees();
        let now = Instant::now();
        manager
            .dispatch_event(&moved(15.0, 15.0), &mut (), &trees, now)
            .await;
        for _ in 0..3 {
            for state in [ElementState::Pressed, ElementState::Released] {
                (manager.dispatch_event(&mouse(state), &mut (), &trees, now)).await;
            }
        }
        let chart: Vec<_> = (heard(&log).into_iter())
            .filter(|(name, ..)| *name == "chart")
            .map(|(_, event, _)| event)
            .collect();
        let (click, double) = (SceneGraphEventType::Click, SceneGraphEventType::DoubleClick);
        assert_eq!(chart, [click.clone(), double, click]);
    });
}

/// Requests for one scene at different levels merge to the higher level, in either order.
#[test]
fn requests_for_a_scene_merge_to_the_higher_level() {
    assert_eq!(
        Rebuild::tree("chart").and_scene("chart"),
        Rebuild::tree("chart")
    );
    assert_eq!(
        Rebuild::scene("chart").and_tree("chart"),
        Rebuild::tree("chart")
    );
}
