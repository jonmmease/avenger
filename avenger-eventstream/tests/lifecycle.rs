use std::sync::Arc;

use async_trait::async_trait;
use avenger_common::time::{Duration, Instant};
use avenger_eventstream::{
    manager::{EventStreamHandler, EventStreamManager},
    runtime::{DebounceConfig, RuntimeHostCommand, RuntimeWakeEvent},
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{
        BetweenLifecycle, EventAdmission, EventStreamConfig, EventStreamContext, EventStreamFilter,
        EventStreamPhase, UpdateStatus,
    },
    window::{ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;

#[derive(Clone, Default)]
struct State {
    deliveries: Vec<(&'static str, EventStreamContext)>,
}

struct Recorder {
    name: &'static str,
    admission: Option<EventAdmission>,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<State> for Recorder {
    async fn handle(
        &self,
        _: &SceneGraphEvent,
        _: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        unreachable!("manager supplies context")
    }

    async fn handle_with_context(
        &self,
        _: &SceneGraphEvent,
        context: &EventStreamContext,
        state: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        state.deliveries.push((self.name, context.clone()));
        UpdateStatus {
            admission: self.admission,
            ..Default::default()
        }
    }
}

struct Harness {
    manager: EventStreamManager<State>,
    tree: SceneGraphRTree,
    start: Instant,
}
impl Harness {
    fn new() -> Self {
        Self {
            manager: EventStreamManager::new(State::default()),
            tree: SceneGraphRTree::from_scene_graph(&SceneGraph {
                marks: vec![],
                width: 100.,
                height: 100.,
                origin: [0., 0.],
            }),
            start: Instant::now(),
        }
    }

    fn register(&mut self, name: &'static str, config: EventStreamConfig) {
        self.manager.register_handler(
            config,
            Arc::new(Recorder {
                name,
                admission: None,
            }),
        );
    }

    async fn send(&mut self, event: WindowEvent, millis: u64) -> UpdateStatus {
        self.manager
            .dispatch_event(
                &event,
                &self.tree,
                self.start + Duration::from_millis(millis),
            )
            .await
    }

    async fn move_to(&mut self, x: f32, millis: u64) -> UpdateStatus {
        self.send(
            WindowEvent::CursorMoved(WindowCursorMoved { position: [x, 2.] }),
            millis,
        )
        .await
    }

    async fn press(&mut self, x: f32, millis: u64) {
        self.move_to(x, millis).await;
        self.send(mouse(ElementState::Pressed), millis).await;
    }

    fn contexts(&self) -> Vec<&EventStreamContext> {
        self.manager
            .state()
            .deliveries
            .iter()
            .map(|(_, c)| c)
            .collect()
    }
}

fn mouse(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

fn trigger(event_type: SceneGraphEventType) -> EventStreamConfig {
    EventStreamConfig {
        types: vec![event_type],
        ..Default::default()
    }
}

fn drag() -> EventStreamConfig {
    EventStreamConfig {
        types: vec![SceneGraphEventType::CursorMoved],
        between: Some((
            Box::new(trigger(SceneGraphEventType::MouseDown)),
            Box::new(trigger(SceneGraphEventType::MouseUp)),
        )),
        between_lifecycle: Some(BetweenLifecycle {
            cancel: Some(EventStreamFilter::context(|event, context, _| {
                context.start_position().is_some()
                    && matches!(event, SceneGraphEvent::WindowFocused(false))
            })),
        }),
        ..Default::default()
    }
}

fn wake(status: &UpdateStatus) -> RuntimeWakeEvent {
    status
        .commands
        .iter()
        .find_map(|command| {
            if let RuntimeHostCommand::RequestWakeup {
                key, generation, ..
            } = command
            {
                Some(RuntimeWakeEvent {
                    key: key.clone(),
                    generation: *generation,
                })
            } else {
                None
            }
        })
        .expect("scheduled update")
}

#[tokio::test]
async fn finish_flushes_latest_update_then_notifies_and_invalidates_wakes() {
    for positional_end in [false, true] {
        let mut h = Harness::new();
        let mut config = drag();
        config.debounce = Some(DebounceConfig::new(20));
        if !positional_end {
            *config.between.as_mut().unwrap().1 =
                trigger(SceneGraphEventType::WindowCloseRequested);
        }
        h.register("drag", config);
        h.register("wake observer", trigger(SceneGraphEventType::RuntimeWake));
        h.press(1., 0).await;
        let first = wake(&h.move_to(3., 1).await);
        let last = wake(&h.move_to(5., 2).await);
        assert!(h.contexts().is_empty());
        let end = if positional_end {
            mouse(ElementState::Released)
        } else {
            WindowEvent::WindowCloseRequested
        };
        let status = h.send(end, 3).await;
        let contexts = h.contexts();
        assert_eq!(contexts.len(), 2);
        assert_eq!(contexts[0].phase, EventStreamPhase::Update);
        assert_eq!(contexts[0].current_position(), Some([5., 2.]));
        assert_eq!(contexts[0].delta_from_start(), Some([4., 0.]));
        assert_eq!(contexts[1].phase, EventStreamPhase::Finish);
        assert_eq!(contexts[1].start_position(), Some([1., 2.]));
        assert_eq!(contexts[1].previous_position(), Some([5., 2.]));
        assert_eq!(
            contexts[1].current_position(),
            positional_end.then_some([5., 2.])
        );
        assert!(status.commands.contains(&RuntimeHostCommand::CancelWakeup {
            key: last.key.clone()
        }));
        h.send(WindowEvent::RuntimeWake(first), 30).await;
        h.send(WindowEvent::RuntimeWake(last), 31).await;
        h.move_to(7., 32).await;
        assert_eq!(h.contexts().len(), 2);
        h.press(10., 33).await;
        let next = wake(&h.move_to(12., 34).await);
        h.send(WindowEvent::RuntimeWake(next), 54).await;
        let fresh = h.contexts()[2];
        assert_eq!(fresh.start_position(), Some([10., 2.]));
        assert_eq!(fresh.previous_position(), None);
    }
}

#[tokio::test]
async fn cancellation_discards_pending_updates_and_preserves_accepted_history_in_notification() {
    let mut h = Harness::new();
    h.register(
        "drag",
        EventStreamConfig {
            debounce: Some(DebounceConfig::new(20)),
            ..drag()
        },
    );
    h.press(1., 0).await;
    let accepted = wake(&h.move_to(3., 1).await);
    h.send(WindowEvent::RuntimeWake(accepted), 21).await;
    let pending = wake(&h.move_to(5., 22).await);
    h.send(WindowEvent::WindowFocused(true), 23).await;
    assert_eq!(h.contexts().len(), 1);
    let canceled = h.send(WindowEvent::WindowFocused(false), 24).await;
    assert!(canceled
        .commands
        .contains(&RuntimeHostCommand::CancelWakeup {
            key: pending.key.clone()
        }));
    let contexts = h.contexts();
    assert_eq!(contexts.len(), 2);
    assert_eq!(contexts[1].phase, EventStreamPhase::Cancel);
    assert_eq!(contexts[1].start_position(), Some([1., 2.]));
    assert_eq!(contexts[1].previous_position(), Some([3., 2.]));
    assert_eq!(contexts[1].current_position(), None);
    assert_eq!(contexts[1].delta_from_start(), None);
    h.send(WindowEvent::RuntimeWake(pending.clone()), 50).await;
    h.send(WindowEvent::WindowFocused(false), 51).await;
    h.press(20., 52).await;
    let next = wake(&h.move_to(22., 53).await);
    h.send(WindowEvent::RuntimeWake(pending), 54).await;
    assert_eq!(h.contexts().len(), 2);
    h.send(WindowEvent::RuntimeWake(next), 73).await;
    assert_eq!(h.contexts()[2].start_position(), Some([20., 2.]));
    assert_eq!(h.contexts()[2].previous_position(), None);
}

#[tokio::test]
async fn terminals_bypass_update_filters_and_throttle_and_reset_throttle() {
    let mut h = Harness::new();
    h.register(
        "drag",
        EventStreamConfig {
            filter: Some(vec![EventStreamFilter::event(|e| {
                e.position().is_some_and(|p| p[0] < 5.)
            })]),
            throttle: Some(100),
            ..drag()
        },
    );
    h.press(1., 0).await;
    h.move_to(3., 1).await;
    h.move_to(4., 2).await;
    h.move_to(6., 3).await;
    h.send(mouse(ElementState::Released), 4).await;
    let contexts = h.contexts();
    assert_eq!(contexts.len(), 2);
    assert_eq!(contexts[1].phase, EventStreamPhase::Finish);
    assert_eq!(contexts[1].current_position(), Some([6., 2.]));
    assert_eq!(contexts[1].delta_from_previous(), Some([3., 0.]));
    h.press(1., 5).await;
    h.move_to(2., 6).await;
    assert_eq!(h.contexts().len(), 3);
    assert_eq!(h.contexts()[2].previous_position(), None);
}

#[tokio::test]
async fn cancellation_precedes_end_and_reaches_all_active_streams_after_consumption() {
    let mut h = Harness::new();
    h.register(
        "consumer",
        EventStreamConfig {
            consume: true,
            ..trigger(SceneGraphEventType::WindowFocused)
        },
    );
    for name in ["first", "second"] {
        let mut config = drag();
        *config.between.as_mut().unwrap().1 = trigger(SceneGraphEventType::WindowFocused);
        config.debounce = Some(DebounceConfig::new(20));
        h.register(name, config);
    }
    h.register("ordinary", trigger(SceneGraphEventType::WindowFocused));
    h.press(1., 0).await;
    let pending = h.move_to(2., 1).await;
    let result = h.send(WindowEvent::WindowFocused(false), 2).await;
    assert!(result.consume);
    let deliveries = &h.manager.state().deliveries;
    assert_eq!(
        deliveries.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        ["consumer", "first", "second"]
    );
    assert!(deliveries[1..]
        .iter()
        .all(|(_, c)| c.phase == EventStreamPhase::Cancel));
    for command in pending.commands {
        if let RuntimeHostCommand::RequestWakeup {
            key, generation, ..
        } = command
        {
            h.send(
                WindowEvent::RuntimeWake(RuntimeWakeEvent { key, generation }),
                30,
            )
            .await;
        }
    }
    assert_eq!(h.contexts().len(), 3);
}

#[tokio::test]
async fn rejection_or_failure_cannot_revive_a_finished_or_canceled_session() {
    for admission in [EventAdmission::Rejected, EventAdmission::Failed] {
        for cancel in [false, true] {
            let mut h = Harness::new();
            h.manager.register_handler(
                EventStreamConfig {
                    debounce: Some(DebounceConfig::new(20)),
                    ..drag()
                },
                Arc::new(Recorder {
                    name: "reject",
                    admission: Some(admission),
                }),
            );
            h.press(1., 0).await;
            let pending = wake(&h.move_to(2., 1).await);
            let end = if cancel {
                WindowEvent::WindowFocused(false)
            } else {
                mouse(ElementState::Released)
            };
            h.send(end, 2).await;
            let count = h.contexts().len();
            assert_eq!(count, if cancel { 1 } else { 2 });
            let terminal = h.contexts()[count - 1];
            assert_eq!(
                terminal.phase,
                if cancel {
                    EventStreamPhase::Cancel
                } else {
                    EventStreamPhase::Finish
                }
            );
            assert_eq!(terminal.previous_position(), None);
            h.send(WindowEvent::RuntimeWake(pending), 30).await;
            h.move_to(3., 31).await;
            assert_eq!(h.contexts().len(), count);
            h.press(10., 32).await;
            h.move_to(11., 33).await;
            h.send(mouse(ElementState::Released), 34).await;
            assert_eq!(
                h.contexts().last().unwrap().start_position(),
                Some([10., 2.])
            );
        }
    }
}

#[tokio::test]
async fn legacy_end_preserves_delayed_update_and_has_no_terminal_notification() {
    let mut h = Harness::new();
    h.register(
        "legacy",
        EventStreamConfig {
            between_lifecycle: None,
            debounce: Some(DebounceConfig::new(20)),
            ..drag()
        },
    );
    h.press(1., 0).await;
    let pending = wake(&h.move_to(2., 1).await);
    h.send(mouse(ElementState::Released), 2).await;
    assert!(h.contexts().is_empty());
    h.send(WindowEvent::RuntimeWake(pending), 21).await;
    assert_eq!(h.contexts().len(), 1);
    assert_eq!(h.contexts()[0].phase, EventStreamPhase::Update);
    assert_eq!(h.contexts()[0].start_position(), Some([1., 2.]));
}

#[tokio::test]
async fn lifecycle_without_between_leaves_ordinary_debounce_unchanged() {
    let mut h = Harness::new();
    h.register(
        "ordinary",
        EventStreamConfig {
            between: None,
            debounce: Some(DebounceConfig::new(20)),
            ..drag()
        },
    );
    let pending = wake(&h.move_to(2., 1).await);
    h.send(WindowEvent::WindowFocused(false), 2).await;
    h.send(WindowEvent::RuntimeWake(pending), 21).await;
    assert_eq!(h.contexts().len(), 1);
    assert_eq!(h.contexts()[0].phase, EventStreamPhase::Update);
    assert_eq!(h.contexts()[0].start_position(), None);
    assert_eq!(h.contexts()[0].delta_from_start(), None);
    assert_eq!(h.contexts()[0].delta_from_previous(), None);
}
