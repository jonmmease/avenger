//! Streams hear events in scenes, from the top scene down. A stream that consumes an event stops
//! it, a pointer event stops at a scene whose interactive mark lies under it, any other event
//! falls through every scene, and the scene where a press stops owns its moves and release.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_common::time::Instant;
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, Rebuild};
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

/// Records what it hears under its name, in its own scene's view of the trees, and asks for a
/// new scene named after itself.
struct Record(&'static str, usize, Heard);

#[async_trait]
impl EventStreamHandler<()> for Record {
    async fn handle(&self, event: &SceneGraphEvent, _: &mut (), trees: &SceneTrees<'_>) -> Rebuild {
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

fn on(types: &[SceneGraphEventType]) -> EventStreamConfig {
    EventStreamConfig {
        types: types.to_vec(),
        ..Default::default()
    }
}

/// The tree of a scene with one rect, `name`, from `min` to `max`.
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

/// A chart on the bottom scene, from (0, 0) to (20, 20), under a box on the top scene, from
/// (0, 0) to (10, 10).
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

/// A manager with a "chart" stream on the bottom scene and a "box" stream on the top one, both
/// on `types`.
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

/// A pointer event stops at a scene whose interactive mark lies under it, after that scene's
/// streams hear it, and a key falls through every scene, during a press too.
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

/// A press keeps its moves and its release with the scenes that heard it: a drag that starts on
/// the chart keeps reaching it across the box, and one that starts on the box never reaches it.
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

/// A window that loses focus cancels its press, whose release may never come: the press's owner
/// hears `MouseCancel`, the scenes below it don't, and the moves after it reach every scene
/// again.
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

/// A stream that consumes an event keeps it from the scenes below, except from a press's owner,
/// which hears the press's moves and release whatever the scenes above it consume, so a drag that
/// starts on the chart ends there, even over a box that takes those events.
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
        let consuming = EventStreamConfig {
            consume: true,
            ..on(&types)
        };
        manager.register_handler(1, consuming, Arc::new(Record("box", 1, log.clone())));
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

/// A canceled press closes the windows of the streams that it opened, as its release would, so
/// a drag doesn't go on without its press.
#[test]
fn a_cancelled_press_closes_its_drag() {
    block_on(async {
        let log = Heard::default();
        let mut manager = EventStreamManager::new();
        let drag = EventStreamConfig {
            between: Some((
                Box::new(on(&[SceneGraphEventType::MouseDown])),
                Box::new(on(&[SceneGraphEventType::MouseUp])),
            )),
            ..on(&[SceneGraphEventType::CursorMoved])
        };
        manager.register_handler(0, drag, Arc::new(Record("drag", 0, log.clone())));
        let trees = trees();
        let now = Instant::now();
        manager
            .dispatch_event(&moved(15.0, 15.0), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&mouse(ElementState::Pressed), &mut (), &trees, now)
            .await;
        manager
            .dispatch_event(&moved(16.0, 16.0), &mut (), &trees, now)
            .await;
        assert_eq!(heard(&log).len(), 1);
        for focused in [false, true] {
            manager
                .dispatch_event(&WindowEvent::WindowFocused(focused), &mut (), &trees, now)
                .await;
        }
        manager
            .dispatch_event(&moved(17.0, 17.0), &mut (), &trees, now)
            .await;
        assert_eq!(heard(&log), []);
    });
}

/// Each scene's streams hear mark-enter and mark-leave events of their own tree's marks, and a
/// scene under an interactive mark of a scene above has no mark under the pointer.
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
