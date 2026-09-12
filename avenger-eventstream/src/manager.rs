use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use avenger_common::time::{Duration, Instant};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;

use crate::{
    runtime::RuntimeWakeKey,
    scene::{
        ModifiersState, SceneClickEvent, SceneCursorMovedEvent, SceneDoubleClickEvent,
        SceneFileChangedEvent, SceneGraphEvent, SceneGraphEventType, SceneKeyPressEvent,
        SceneKeyReleaseEvent, SceneMouseDownEvent, SceneMouseEnterEvent, SceneMouseLeaveEvent,
        SceneMouseUpEvent, SceneMouseWheelEvent,
    },
    stream::{EventAdmission, EventStream, EventStreamConfig, EventStreamContext, UpdateStatus},
    window::{ElementState, Key, MouseButton, NamedKey, WindowEvent, WindowKeyboardInput},
};

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait EventStreamHandler<State: Clone + Send + Sync + 'static>: Send + Sync {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus;

    async fn handle_with_context(
        &self,
        event: &SceneGraphEvent,
        _context: &EventStreamContext,
        state: &mut State,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        self.handle(event, state, rtree).await
    }
}

#[derive(Clone)]
pub struct EventStreamManager<State: Clone + Send + Sync + 'static> {
    state: State,
    streams: Vec<EventStream<State>>,
    current_mark: Option<MarkInstance>,
    last_click: Option<(Instant, [f32; 2], Option<MarkInstance>)>,
    double_click_threshold: Duration,
    // Double-click distance threshold (e.g., 5 pixels)
    double_click_distance: f32,
    // Track current cursor position
    current_cursor_position: Option<[f32; 2]>,
    // Track current mousedown mark, used for click determination
    mousedown_mark: Option<MarkInstance>,
    mousedown_button: Option<MouseButton>,
    suppress_click: bool,
    modifiers: ModifiersState,
}

impl<State: Clone + Send + Sync + 'static> EventStreamManager<State> {
    pub fn new(state: State) -> Self {
        Self {
            state,
            streams: Vec::new(),
            current_mark: None,
            last_click: None,
            double_click_threshold: Duration::from_millis(500),
            double_click_distance: 5.0,
            current_cursor_position: None,
            mousedown_mark: None,
            mousedown_button: None,
            suppress_click: false,
            modifiers: ModifiersState::default(),
        }
    }

    /// Register a new event handler with the given configuration
    pub fn register_handler(
        &mut self,
        config: EventStreamConfig,
        handler: Arc<dyn EventStreamHandler<State>>,
    ) {
        let wake_key = RuntimeWakeKey::new(
            "event-stream-manager",
            0,
            format!("stream-{}-debounce", self.streams.len()),
        );
        let stream = EventStream::new_with_wake_key(config, handler, wake_key);
        self.streams.push(stream);
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    fn update_modifiers(&mut self, input: &WindowKeyboardInput) {
        match (input.key, input.state) {
            (Key::Named(NamedKey::Shift), ElementState::Pressed) => self.modifiers.shift = true,
            (Key::Named(NamedKey::Shift), ElementState::Released) => self.modifiers.shift = false,
            (Key::Named(NamedKey::Control), ElementState::Pressed) => self.modifiers.control = true,
            (Key::Named(NamedKey::Control), ElementState::Released) => {
                self.modifiers.control = false
            }
            (Key::Named(NamedKey::Alt), ElementState::Pressed) => self.modifiers.alt = true,
            (Key::Named(NamedKey::Alt), ElementState::Released) => self.modifiers.alt = false,
            (Key::Named(NamedKey::Super), ElementState::Pressed) => self.modifiers.meta = true,
            (Key::Named(NamedKey::Super), ElementState::Released) => self.modifiers.meta = false,
            _ => {}
        }
    }

    /// Get all file paths that are watched by registered event streams
    pub fn get_watched_files(&self) -> Vec<PathBuf> {
        self.streams
            .iter()
            .flat_map(|stream| {
                stream
                    .config
                    .types
                    .iter()
                    .filter_map(|t| match t {
                        SceneGraphEventType::FileChanged(file_path) => Some(file_path.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub async fn dispatch_event(
        &mut self,
        event: &WindowEvent,
        rtree: &SceneGraphRTree,
        instant: Instant,
    ) -> UpdateStatus {
        // Update modifier state based on keyboard events
        if let Some(input) = event.keyboard_input() {
            self.update_modifiers(input);
        }
        if let WindowEvent::ModifiersChanged(modifiers) = event {
            self.modifiers = *modifiers;
        }
        if matches!(event, WindowEvent::WindowFocused(false)) {
            // Key releases can go to the newly focused window.
            self.modifiers = ModifiersState::default();
        }

        // Update cursor position tracking
        if let Some(position) = event.position() {
            self.current_cursor_position = Some(position);
        }

        let mut update_status = UpdateStatus::default();

        // Cursor exit and focus loss have no position, so the ordinary hover
        // transition path below cannot synthesize the final mark-leave event.
        // Use the last logical position and clear hover ownership exactly once.
        if matches!(
            event,
            WindowEvent::CursorLeft | WindowEvent::WindowFocused(false)
        ) {
            update_status = update_status.merge(&self.leave_current_mark(rtree, instant).await);
            self.current_cursor_position = None;
            self.mousedown_mark = None;
            self.mousedown_button = None;
        }

        // Convert window event to scene graph event
        let scene_event = match event {
            WindowEvent::MouseInput(input) => {
                if let Some(position) = self.current_cursor_position {
                    let mark_instance = self.get_mark_path_for_event_at_position(&position, rtree);

                    if input.state == ElementState::Pressed {
                        // Store both mark and button
                        self.mousedown_mark = mark_instance.clone();
                        self.mousedown_button = Some(input.button);
                        Some(SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                            position,
                            button: input.button,
                            mark_instance,
                            modifiers: self.modifiers,
                        }))
                    } else if input.state == ElementState::Released {
                        // Check if both mark and button match
                        if !self.suppress_click
                            && self.mousedown_mark.as_ref() == mark_instance.as_ref()
                            && self.mousedown_button.as_ref() == Some(&input.button)
                        {
                            if input.button == MouseButton::Left {
                                update_status = update_status.merge(
                                    &self
                                        .check_double_click(
                                            position,
                                            mark_instance.clone(),
                                            rtree,
                                            instant,
                                        )
                                        .await,
                                );
                            } else {
                                update_status = update_status.merge(
                                    &self
                                        .dispatch_single_event(
                                            &SceneGraphEvent::Click(SceneClickEvent {
                                                position,
                                                button: input.button,
                                                mark_instance: mark_instance.clone(),
                                                modifiers: self.modifiers,
                                            }),
                                            rtree,
                                            instant,
                                            None,
                                        )
                                        .await,
                                );
                            }
                        }
                        self.suppress_click = false;
                        self.mousedown_mark = None;
                        self.mousedown_button = None;
                        Some(SceneGraphEvent::MouseUp(SceneMouseUpEvent {
                            position,
                            button: input.button,
                            mark_instance,
                            modifiers: self.modifiers,
                        }))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            WindowEvent::CursorMoved(e) => {
                let mark_instance = self.get_mark_path_for_event_at_position(&e.position, rtree);
                Some(SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: e.position,
                    mark_instance,
                    modifiers: self.modifiers,
                }))
            }
            WindowEvent::MouseWheel(e) => {
                if let Some(position) = self.current_cursor_position {
                    let mark_instance = self.get_mark_path_for_event_at_position(&position, rtree);
                    Some(SceneGraphEvent::MouseWheel(SceneMouseWheelEvent {
                        position,
                        delta: e.delta,
                        mark_instance,
                        modifiers: self.modifiers,
                    }))
                } else {
                    None
                }
            }
            WindowEvent::KeyboardInput(e) => {
                let position = self.current_cursor_position;
                let mark_instance =
                    position.and_then(|p| self.get_mark_path_for_event_at_position(&p, rtree));
                if e.state == ElementState::Pressed {
                    Some(SceneGraphEvent::KeyPress(SceneKeyPressEvent {
                        position,
                        key: e.key,
                        text: e.text.clone(),
                        repeat: e.repeat,
                        mark_instance,
                        modifiers: self.modifiers,
                    }))
                } else {
                    Some(SceneGraphEvent::KeyRelease(SceneKeyReleaseEvent {
                        position,
                        key: e.key,
                        mark_instance,
                        modifiers: self.modifiers,
                    }))
                }
            }
            WindowEvent::TextInput(input) => Some(SceneGraphEvent::TextInput {
                input: input.clone(),
                modifiers: self.modifiers,
            }),
            WindowEvent::PointerCaptureLost => Some(SceneGraphEvent::PointerCaptureLost),
            WindowEvent::FocusEntered { reverse } => {
                Some(SceneGraphEvent::FocusEntered { reverse: *reverse })
            }
            WindowEvent::Ime(event) => Some(SceneGraphEvent::Ime(event.clone())),
            WindowEvent::Clipboard(event) => Some(SceneGraphEvent::Clipboard(event.clone())),
            WindowEvent::RuntimeWake(event) => Some(SceneGraphEvent::RuntimeWake(event.clone())),
            WindowEvent::WindowResize(e) => Some(SceneGraphEvent::WindowResize(e.clone())),
            WindowEvent::WindowResizeSettled(e) => {
                Some(SceneGraphEvent::WindowResizeSettled(e.clone()))
            }
            WindowEvent::CanvasResize(e) => Some(SceneGraphEvent::CanvasResize(e.clone())),
            WindowEvent::CanvasResizeSettled(e) => {
                Some(SceneGraphEvent::CanvasResizeSettled(e.clone()))
            }
            WindowEvent::WindowMoved(e) => Some(SceneGraphEvent::WindowMoved(e.clone())),
            WindowEvent::WindowFocused(focused) => Some(SceneGraphEvent::WindowFocused(*focused)),
            WindowEvent::WindowCloseRequested => Some(SceneGraphEvent::WindowCloseRequested),
            WindowEvent::InteractionSettled { .. } => Some(SceneGraphEvent::InteractionSettled),
            WindowEvent::FileChanged(e) => {
                Some(SceneGraphEvent::FileChanged(SceneFileChangedEvent {
                    file_path: e.file_path.clone(),
                    error: e.error.clone(),
                }))
            }
            _ => None,
        };

        // Process cursor movement for enter/leave events
        if let Some(position) = event.position() {
            update_status = update_status.merge(
                &self
                    .handle_mark_mouse_events(position, rtree, instant)
                    .await,
            );
        }

        // Dispatch the converted event if any
        if let Some(scene_event) = scene_event {
            update_status = update_status.merge(
                &self
                    .dispatch_single_event(&scene_event, rtree, instant, None)
                    .await,
            );
        }

        if matches!(event, WindowEvent::MouseInput(input) if input.state == ElementState::Pressed) {
            self.suppress_click = update_status.suppress_click;
            if self.suppress_click {
                self.last_click = None;
            }
        }
        update_status
    }

    async fn leave_current_mark(
        &mut self,
        rtree: &SceneGraphRTree,
        instant: Instant,
    ) -> UpdateStatus {
        let Some(previous) = self.current_mark.take() else {
            return UpdateStatus::default();
        };
        let position = self.current_cursor_position.unwrap_or([0.0, 0.0]);
        self.dispatch_single_event(
            &SceneGraphEvent::MouseLeave(SceneMouseLeaveEvent {
                position,
                mark_instance: previous.clone(),
                modifiers: self.modifiers,
            }),
            rtree,
            instant,
            Some(previous),
        )
        .await
    }

    async fn dispatch_single_event(
        &mut self,
        event: &SceneGraphEvent,
        rtree: &SceneGraphRTree,
        instant: Instant,
        mark_instance: Option<MarkInstance>,
    ) -> UpdateStatus {
        let mark_instance = mark_instance.or_else(|| self.get_mark_path_for_event(event, rtree));

        let mut update_status = UpdateStatus::default();

        // Private debounce wakeups never reach application subscribers, including
        // stale generations. Application-owned keys follow ordinary event matching.
        if let SceneGraphEvent::RuntimeWake(wake) = event {
            if let Some(stream) = self
                .streams
                .iter_mut()
                .find(|stream| stream.owns_runtime_wakeup(&wake.key))
            {
                let debounced = stream
                    .handle_runtime_wakeup(wake, instant)
                    .expect("a debounce owner has a timer");
                update_status.commands.extend(debounced.commands);
                if let Some(ready) = debounced.commit {
                    let handled = stream
                        .handler
                        .handle_with_context(&ready.event, &ready.context, &mut self.state, rtree)
                        .await;
                    if handled.admission == Some(EventAdmission::Rejected)
                        || handled.admission == Some(EventAdmission::Failed)
                    {
                        stream.restore_admission_checkpoint(ready.checkpoint);
                    } else {
                        stream.mark_accepted(
                            &ready.event,
                            ready.context.mark_instance.as_ref(),
                            instant,
                        );
                    }
                    update_status = update_status.merge(&handled);
                }
                return update_status;
            }
        }

        for stream in &mut self.streams {
            let checkpoint = stream.admission_checkpoint();
            let Some(context) =
                stream.matches_and_update(event, mark_instance.as_ref(), rtree, instant)
            else {
                continue;
            };

            let ready_event = if let Some(debounced) =
                stream.debounce_submission(event, context.clone(), checkpoint.clone(), instant)
            {
                update_status.commands.extend(debounced.commands);
                debounced.commit
            } else {
                Some(crate::stream::DebouncedEvent {
                    event: event.clone(),
                    context,
                    checkpoint: checkpoint.clone(),
                })
            };

            let queued = ready_event.is_none();
            let dynamically_consumed = if let Some(ready) = ready_event {
                let handled = stream
                    .handler
                    .handle_with_context(&ready.event, &ready.context, &mut self.state, rtree)
                    .await;
                let committed = !matches!(
                    handled.admission,
                    Some(EventAdmission::Rejected | EventAdmission::Failed)
                );
                if !committed {
                    stream.restore_admission_checkpoint(checkpoint);
                }
                let consume = committed && handled.consume;
                update_status = update_status.merge(&handled);
                (consume, committed)
            } else {
                (false, false)
            };

            let (dynamically_consumed, committed) = dynamically_consumed;
            if committed {
                stream.mark_accepted(event, mark_instance.as_ref(), instant);
            }

            // Debounced streams consume the originating event at match time,
            // even when their handler runs later on the wake-up.
            if ((queued || committed) && stream.config.consume) || dynamically_consumed {
                update_status.consume = true;
                break;
            }
        }

        update_status
    }

    fn get_mark_path_for_event(
        &self,
        event: &SceneGraphEvent,
        rtree: &SceneGraphRTree,
    ) -> Option<MarkInstance> {
        event
            .position()
            .and_then(|pos| self.get_mark_path_for_event_at_position(&pos, rtree))
    }

    fn get_mark_path_for_event_at_position(
        &self,
        position: &[f32; 2],
        rtree: &SceneGraphRTree,
    ) -> Option<MarkInstance> {
        rtree.pick_top_mark_at_point(position).cloned()
    }

    async fn handle_mark_mouse_events(
        &mut self,
        position: [f32; 2],
        rtree: &SceneGraphRTree,
        instant: Instant,
    ) -> UpdateStatus {
        let current_mark = self.get_mark_path_for_event_at_position(&position, rtree);

        let mut update_status = UpdateStatus::default();
        // Handle mark enter/leave
        match (&self.current_mark, &current_mark) {
            (Some(prev), Some(curr)) if prev != curr => {
                // Mark changed - generate leave then enter
                // Use the previous mark instance for leave event
                update_status = update_status.merge(
                    &self
                        .dispatch_single_event(
                            &SceneGraphEvent::MouseLeave(SceneMouseLeaveEvent {
                                position,
                                mark_instance: prev.clone(),
                                modifiers: self.modifiers,
                            }),
                            rtree,
                            instant,
                            Some(prev.clone()),
                        )
                        .await,
                );
                // Use the current mark instance for enter event
                update_status = update_status.merge(
                    &self
                        .dispatch_single_event(
                            &SceneGraphEvent::MouseEnter(SceneMouseEnterEvent {
                                position,
                                mark_instance: curr.clone(),
                                modifiers: self.modifiers,
                            }),
                            rtree,
                            instant,
                            Some(curr.clone()),
                        )
                        .await,
                );
            }
            (Some(prev), None) => {
                // Left mark - generate leave
                update_status = update_status.merge(
                    &self
                        .dispatch_single_event(
                            &SceneGraphEvent::MouseLeave(SceneMouseLeaveEvent {
                                position,
                                mark_instance: prev.clone(),
                                modifiers: self.modifiers,
                            }),
                            rtree,
                            instant,
                            Some(prev.clone()),
                        )
                        .await,
                );
            }
            (None, Some(curr)) => {
                // Entered mark - generate enter
                update_status = update_status.merge(
                    &self
                        .dispatch_single_event(
                            &SceneGraphEvent::MouseEnter(SceneMouseEnterEvent {
                                position,
                                mark_instance: curr.clone(),
                                modifiers: self.modifiers,
                            }),
                            rtree,
                            instant,
                            Some(curr.clone()),
                        )
                        .await,
                );
            }
            _ => {}
        }

        // Update current mark
        self.current_mark = current_mark;

        update_status
    }

    async fn check_double_click(
        &mut self,
        position: [f32; 2],
        mark_instance: Option<MarkInstance>,
        rtree: &SceneGraphRTree,
        instant: Instant,
    ) -> UpdateStatus {
        let mut update_status = UpdateStatus::default();

        let is_double_click = if let Some((last_time, last_pos, last_mark)) = &self.last_click {
            let time_diff = instant.duration_since(*last_time);
            let distance =
                ((position[0] - last_pos[0]).powi(2) + (position[1] - last_pos[1]).powi(2)).sqrt();

            time_diff <= self.double_click_threshold
                && distance <= self.double_click_distance
                && last_mark.as_ref() == mark_instance.as_ref()
        } else {
            false
        };

        // Always emit the underlying click. DoubleClick is an additional higher-level event,
        // not a replacement for the second click in the pair.
        update_status = update_status.merge(
            &self
                .dispatch_single_event(
                    &SceneGraphEvent::Click(SceneClickEvent {
                        position,
                        button: MouseButton::Left,
                        mark_instance: mark_instance.clone(),
                        modifiers: self.modifiers,
                    }),
                    rtree,
                    instant,
                    None,
                )
                .await,
        );

        if is_double_click {
            update_status = update_status.merge(
                &self
                    .dispatch_single_event(
                        &SceneGraphEvent::DoubleClick(SceneDoubleClickEvent {
                            position,
                            mark_instance,
                            modifiers: self.modifiers,
                        }),
                        rtree,
                        instant,
                        None,
                    )
                    .await,
            );
            self.last_click = None;
        } else {
            self.last_click = Some((instant, position, mark_instance));
        }

        update_status
    }

    pub fn modifiers(&self) -> ModifiersState {
        self.modifiers
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use avenger_common::value::ScalarOrArray;
    use avenger_scenegraph::{marks::symbol::SceneSymbolMark, scene_graph::SceneGraph};

    use super::*;
    use crate::{
        runtime::{RuntimeHostCommand, RuntimeWakeEvent},
        stream::{DebounceConfig, EventAdmission, EventStreamConfig, EventStreamFilter},
        window::{CanvasResizeEvent, WindowCursorMoved, WindowEvent, WindowMouseInput},
    };

    #[derive(Clone, Default)]
    struct TestState {
        events: Arc<Mutex<Vec<SceneGraphEvent>>>,
        contexts: Arc<Mutex<Vec<EventStreamContext>>>,
        labels: Arc<Mutex<Vec<String>>>,
    }

    struct RecordingHandler;

    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl EventStreamHandler<TestState> for RecordingHandler {
        async fn handle(
            &self,
            event: &SceneGraphEvent,
            state: &mut TestState,
            _rtree: &SceneGraphRTree,
        ) -> UpdateStatus {
            state.events.lock().unwrap().push(event.clone());
            UpdateStatus {
                rerender: true,
                rebuild_geometry: false,
                ..Default::default()
            }
        }
    }

    struct ContextRecordingHandler;

    struct RejectingContextHandler;

    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl EventStreamHandler<TestState> for ContextRecordingHandler {
        async fn handle(
            &self,
            event: &SceneGraphEvent,
            state: &mut TestState,
            _rtree: &SceneGraphRTree,
        ) -> UpdateStatus {
            state.events.lock().unwrap().push(event.clone());
            UpdateStatus {
                rerender: true,
                rebuild_geometry: false,
                ..Default::default()
            }
        }

        async fn handle_with_context(
            &self,
            event: &SceneGraphEvent,
            context: &EventStreamContext,
            state: &mut TestState,
            rtree: &SceneGraphRTree,
        ) -> UpdateStatus {
            state.contexts.lock().unwrap().push(context.clone());
            self.handle(event, state, rtree).await
        }
    }

    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl EventStreamHandler<TestState> for RejectingContextHandler {
        async fn handle(
            &self,
            _event: &SceneGraphEvent,
            _state: &mut TestState,
            _rtree: &SceneGraphRTree,
        ) -> UpdateStatus {
            unreachable!("context path is used")
        }

        async fn handle_with_context(
            &self,
            _event: &SceneGraphEvent,
            context: &EventStreamContext,
            state: &mut TestState,
            _rtree: &SceneGraphRTree,
        ) -> UpdateStatus {
            state.contexts.lock().unwrap().push(context.clone());
            UpdateStatus {
                consume: true,
                admission: Some(EventAdmission::Rejected),
                ..Default::default()
            }
        }
    }

    struct FailingContextHandler;

    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl EventStreamHandler<TestState> for FailingContextHandler {
        async fn handle(
            &self,
            _event: &SceneGraphEvent,
            _state: &mut TestState,
            _rtree: &SceneGraphRTree,
        ) -> UpdateStatus {
            unreachable!("context path is used")
        }

        async fn handle_with_context(
            &self,
            _event: &SceneGraphEvent,
            context: &EventStreamContext,
            state: &mut TestState,
            _rtree: &SceneGraphRTree,
        ) -> UpdateStatus {
            state.contexts.lock().unwrap().push(context.clone());
            UpdateStatus {
                consume: true,
                admission: Some(EventAdmission::Failed),
                ..Default::default()
            }
        }
    }

    #[derive(Clone)]
    struct HandlerId(&'static str);

    #[tokio::test]
    async fn application_wakeups_are_delivered_and_private_wakeups_stay_internal() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::RuntimeWake],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::CanvasResize],
                debounce: Some(DebounceConfig::new(20)),
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );
        let now = Instant::now();
        let wake = RuntimeWakeEvent {
            key: RuntimeWakeKey::new("editor", 1, "caret"),
            generation: 1,
        };
        manager
            .dispatch_event(&WindowEvent::RuntimeWake(wake.clone()), &empty_rtree(), now)
            .await;
        assert!(
            matches!(events.lock().unwrap().as_slice(), [SceneGraphEvent::RuntimeWake(received)] if received == &wake)
        );
        events.lock().unwrap().clear();
        let status = manager
            .dispatch_event(
                &WindowEvent::CanvasResize(CanvasResizeEvent { size: [5.0, 6.0] }),
                &empty_rtree(),
                now,
            )
            .await;
        let RuntimeHostCommand::RequestWakeup {
            key,
            generation,
            deadline,
        } = status.commands[0].clone()
        else {
            panic!("expected wake request")
        };
        manager
            .dispatch_event(
                &WindowEvent::RuntimeWake(RuntimeWakeEvent {
                    key: key.clone(),
                    generation: generation - 1,
                }),
                &empty_rtree(),
                deadline,
            )
            .await;
        assert!(events.lock().unwrap().is_empty());
        manager
            .dispatch_event(
                &WindowEvent::RuntimeWake(RuntimeWakeEvent { key, generation }),
                &empty_rtree(),
                deadline,
            )
            .await;
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [SceneGraphEvent::CanvasResize(_)]
        ));
    }

    #[tokio::test]
    async fn debounced_consuming_stream_stops_propagation_at_admission() {
        let state = TestState::default();
        let labels = state.labels.clone();
        let mut manager = EventStreamManager::new(state);
        let config = EventStreamConfig {
            types: vec![SceneGraphEventType::CanvasResize],
            ..Default::default()
        };
        manager.register_handler(
            EventStreamConfig {
                consume: true,
                debounce: Some(DebounceConfig::new(20)),
                ..config.clone()
            },
            Arc::new(HandlerId("first")),
        );
        manager.register_handler(config, Arc::new(HandlerId("second")));
        let status = manager
            .dispatch_event(
                &WindowEvent::CanvasResize(CanvasResizeEvent { size: [5.0, 6.0] }),
                &empty_rtree(),
                Instant::now(),
            )
            .await;
        assert!(status.consume);
        assert!(labels.lock().unwrap().is_empty());
        let RuntimeHostCommand::RequestWakeup {
            key,
            generation,
            deadline,
        } = status.commands[0].clone()
        else {
            panic!("expected wake request")
        };
        manager
            .dispatch_event(
                &WindowEvent::RuntimeWake(RuntimeWakeEvent { key, generation }),
                &empty_rtree(),
                deadline,
            )
            .await;
        assert_eq!(labels.lock().unwrap().as_slice(), &["first:CanvasResize"]);
    }

    struct DynamicConsumeHandler(&'static str, bool);

    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl EventStreamHandler<TestState> for HandlerId {
        async fn handle(
            &self,
            event: &SceneGraphEvent,
            state: &mut TestState,
            _rtree: &SceneGraphRTree,
        ) -> UpdateStatus {
            state
                .labels
                .lock()
                .unwrap()
                .push(format!("{}:{:?}", self.0, event.event_type()));
            Default::default()
        }
    }

    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl EventStreamHandler<TestState> for DynamicConsumeHandler {
        async fn handle(
            &self,
            _event: &SceneGraphEvent,
            state: &mut TestState,
            _rtree: &SceneGraphRTree,
        ) -> UpdateStatus {
            state.labels.lock().unwrap().push(self.0.to_string());
            UpdateStatus {
                consume: self.1,
                ..Default::default()
            }
        }
    }

    #[tokio::test]
    async fn handler_can_decide_consumption_dynamically_after_accepting_event() {
        let state = TestState::default();
        let labels = state.labels.clone();
        let mut manager = EventStreamManager::new(state);
        let config = EventStreamConfig {
            types: vec![SceneGraphEventType::CanvasResize],
            ..Default::default()
        };
        manager.register_handler(
            config.clone(),
            Arc::new(DynamicConsumeHandler("first", true)),
        );
        manager.register_handler(config, Arc::new(DynamicConsumeHandler("second", false)));

        let status = manager
            .dispatch_event(
                &WindowEvent::CanvasResize(CanvasResizeEvent { size: [5.0, 6.0] }),
                &empty_rtree(),
                Instant::now(),
            )
            .await;
        assert!(status.consume);
        assert_eq!(labels.lock().unwrap().as_slice(), &["first"]);
    }

    #[tokio::test]
    async fn rejected_handler_does_not_advance_previous_or_consume() {
        let state = TestState::default();
        let contexts = state.contexts.clone();
        let labels = state.labels.clone();
        let mut manager = EventStreamManager::new(state);
        let config = EventStreamConfig {
            types: vec![SceneGraphEventType::CanvasResize],
            consume: true,
            ..Default::default()
        };
        manager.register_handler(config.clone(), Arc::new(RejectingContextHandler));
        manager.register_handler(config, Arc::new(HandlerId("following")));

        let start = Instant::now();
        for (offset, size) in [[1.0, 1.0], [2.0, 2.0]].into_iter().enumerate() {
            manager
                .dispatch_event(
                    &WindowEvent::CanvasResize(CanvasResizeEvent { size }),
                    &empty_rtree(),
                    start + Duration::from_millis(offset as u64),
                )
                .await;
        }

        let contexts = contexts.lock().unwrap();
        assert_eq!(contexts.len(), 2);
        assert!(contexts
            .iter()
            .all(|context| context.previous_event.is_none()));
        assert_eq!(labels.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn failed_handler_does_not_advance_previous_or_consume() {
        let state = TestState::default();
        let contexts = state.contexts.clone();
        let labels = state.labels.clone();
        let mut manager = EventStreamManager::new(state);
        let config = EventStreamConfig {
            types: vec![SceneGraphEventType::CanvasResize],
            consume: true,
            ..Default::default()
        };
        manager.register_handler(config.clone(), Arc::new(FailingContextHandler));
        manager.register_handler(config, Arc::new(HandlerId("following")));

        let start = Instant::now();
        for (offset, size) in [[1.0, 1.0], [2.0, 2.0]].into_iter().enumerate() {
            manager
                .dispatch_event(
                    &WindowEvent::CanvasResize(CanvasResizeEvent { size }),
                    &empty_rtree(),
                    start + Duration::from_millis(offset as u64),
                )
                .await;
        }

        let contexts = contexts.lock().unwrap();
        assert_eq!(contexts.len(), 2);
        assert!(contexts
            .iter()
            .all(|context| context.previous_event.is_none()));
        assert_eq!(labels.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn cursor_left_emits_one_leave_and_clears_hover_ownership() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::MarkMouseLeave],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );
        let hovered = test_mark_instance("point", 0);
        manager.current_mark = Some(hovered.clone());
        manager.current_cursor_position = Some([12.0, 34.0]);

        manager
            .dispatch_event(&WindowEvent::CursorLeft, &empty_rtree(), Instant::now())
            .await;
        manager
            .dispatch_event(&WindowEvent::CursorLeft, &empty_rtree(), Instant::now())
            .await;

        assert_eq!(manager.current_mark, None);
        assert_eq!(manager.current_cursor_position, None);
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[SceneGraphEvent::MouseLeave(SceneMouseLeaveEvent {
                position: [12.0, 34.0],
                mark_instance: hovered,
                modifiers: ModifiersState::default(),
            })]
        );
    }

    #[tokio::test]
    async fn cursor_uses_top_hit_without_flicker_and_orders_mark_transitions() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        for event_type in [
            SceneGraphEventType::MarkMouseEnter,
            SceneGraphEventType::MarkMouseLeave,
        ] {
            manager.register_handler(
                EventStreamConfig {
                    types: vec![event_type],
                    ..Default::default()
                },
                Arc::new(RecordingHandler),
            );
        }

        let mut bottom = SceneSymbolMark {
            name: "bottom".to_owned(),
            ..SceneSymbolMark::default()
        };
        bottom.x = ScalarOrArray::new_scalar(10.0);
        bottom.y = ScalarOrArray::new_scalar(10.0);
        let mut top = SceneSymbolMark {
            name: "top".to_owned(),
            ..SceneSymbolMark::default()
        };
        top.x = ScalarOrArray::new_scalar(10.0);
        top.y = ScalarOrArray::new_scalar(10.0);
        let mut next = SceneSymbolMark {
            name: "next".to_owned(),
            ..SceneSymbolMark::default()
        };
        next.x = ScalarOrArray::new_scalar(40.0);
        next.y = ScalarOrArray::new_scalar(10.0);
        let rtree = SceneGraphRTree::from_scene_graph(&SceneGraph {
            marks: vec![bottom.into(), top.into(), next.into()],
            width: 60.0,
            height: 30.0,
            origin: [0.0, 0.0],
        });
        let start = Instant::now();

        for (offset, position) in [[10.0, 10.0], [10.5, 10.0], [40.0, 10.0]]
            .into_iter()
            .enumerate()
        {
            manager
                .dispatch_event(
                    &WindowEvent::CursorMoved(WindowCursorMoved { position }),
                    &rtree,
                    start + Duration::from_millis(offset as u64),
                )
                .await;
        }

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 3, "same-instance movement must not flicker");
        assert!(matches!(
            &events[0],
            SceneGraphEvent::MouseEnter(event) if event.mark_instance.name == "top"
        ));
        assert!(matches!(
            &events[1],
            SceneGraphEvent::MouseLeave(event) if event.mark_instance.name == "top"
        ));
        assert!(matches!(
            &events[2],
            SceneGraphEvent::MouseEnter(event) if event.mark_instance.name == "next"
        ));
    }

    fn empty_rtree() -> SceneGraphRTree {
        SceneGraphRTree::from_scene_graph(&SceneGraph {
            marks: Vec::new(),
            width: 1.0,
            height: 1.0,
            origin: [0.0, 0.0],
        })
    }

    fn test_mark_instance(name: &str, mark_index: usize) -> MarkInstance {
        MarkInstance {
            name: name.to_string(),
            mark_path: vec![mark_index],
            instance_index: Some(0),
        }
    }

    fn left_mouse_down_config() -> EventStreamConfig {
        EventStreamConfig {
            types: vec![SceneGraphEventType::MouseDown],
            filter: Some(vec![EventStreamFilter::event(|event| {
                matches!(
                    event,
                    SceneGraphEvent::MouseDown(mouse_down)
                        if mouse_down.button == MouseButton::Left
                )
            })]),
            ..Default::default()
        }
    }

    fn left_mouse_up_config() -> EventStreamConfig {
        EventStreamConfig {
            types: vec![SceneGraphEventType::MouseUp],
            filter: Some(vec![EventStreamFilter::event(|event| {
                matches!(
                    event,
                    SceneGraphEvent::MouseUp(mouse_up)
                        if mouse_up.button == MouseButton::Left
                )
            })]),
            ..Default::default()
        }
    }

    fn drag_stream_config() -> EventStreamConfig {
        EventStreamConfig {
            types: vec![SceneGraphEventType::CursorMoved],
            between: Some((
                Box::new(left_mouse_down_config()),
                Box::new(left_mouse_up_config()),
            )),
            ..Default::default()
        }
    }

    fn drag_end_emit_stream_config() -> EventStreamConfig {
        EventStreamConfig {
            types: vec![SceneGraphEventType::MouseUp],
            between: Some((
                Box::new(left_mouse_down_config()),
                Box::new(left_mouse_up_config()),
            )),
            emit_between_end_event: true,
            ..Default::default()
        }
    }

    async fn dispatch_cursor(
        manager: &mut EventStreamManager<TestState>,
        position: [f32; 2],
        instant: Instant,
    ) {
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position }),
                &empty_rtree(),
                instant,
            )
            .await;
    }

    async fn dispatch_left_mouse(
        manager: &mut EventStreamManager<TestState>,
        state: ElementState,
        instant: Instant,
    ) {
        manager
            .dispatch_event(
                &WindowEvent::MouseInput(WindowMouseInput {
                    state,
                    button: MouseButton::Left,
                }),
                &empty_rtree(),
                instant,
            )
            .await;
    }

    #[tokio::test]
    async fn canvas_resize_maps_to_scene_graph_event() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::CanvasResize],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );

        let status = manager
            .dispatch_event(
                &WindowEvent::CanvasResize(CanvasResizeEvent {
                    size: [720.0, 420.0],
                }),
                &empty_rtree(),
                Instant::now(),
            )
            .await;

        assert!(status.rerender);
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                size: [720.0, 420.0],
            })]
        );
    }

    #[tokio::test]
    async fn key_press_preserves_full_text_separately_from_logical_key() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::KeyPress],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );

        let now = Instant::now();
        dispatch_cursor(&mut manager, [0.5, 0.5], now).await;
        let text = "e\u{301}🙂";
        manager
            .dispatch_event(
                &WindowEvent::KeyboardInput(WindowKeyboardInput {
                    repeat: false,
                    key: Key::Character('e'),
                    text: Some(text.into()),
                    state: ElementState::Pressed,
                }),
                &empty_rtree(),
                now,
            )
            .await;

        let events = events.lock().unwrap();
        let [SceneGraphEvent::KeyPress(event)] = events.as_slice() else {
            panic!("expected one key-press event");
        };
        assert_eq!(event.key, Key::Character('e'));
        assert_eq!(event.text.as_deref(), Some(text));
    }

    #[tokio::test]
    async fn native_modifier_snapshots_apply_without_modifier_key_events() {
        let mut manager = EventStreamManager::new(TestState::default());
        let event = WindowEvent::from_winit_event(
            winit::event::WindowEvent::ModifiersChanged(
                winit::keyboard::ModifiersState::SUPER.into(),
            ),
            1.0,
        )
        .unwrap();
        manager
            .dispatch_event(&event, &empty_rtree(), Instant::now())
            .await;
        assert!(manager.modifiers().meta);
        manager
            .dispatch_event(
                &WindowEvent::WindowFocused(false),
                &empty_rtree(),
                Instant::now(),
            )
            .await;
        assert_eq!(manager.modifiers(), ModifiersState::default());
        for event in [
            event,
            WindowEvent::WindowFocused(false),
            WindowEvent::WindowCloseRequested,
            WindowEvent::CursorLeft,
        ] {
            assert!(!event.skip_if_render_pending());
        }
    }

    #[tokio::test]
    async fn ime_and_clipboard_events_dispatch_without_a_pointer_position() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::Ime, SceneGraphEventType::Clipboard],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );

        let now = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::Ime(crate::window::ImeEvent::Preedit {
                    text: "かな".into(),
                    cursor: Some((3, 6)),
                }),
                &empty_rtree(),
                now,
            )
            .await;
        manager
            .dispatch_event(
                &WindowEvent::Clipboard(crate::window::ClipboardEvent::Paste("pasted".into())),
                &empty_rtree(),
                now,
            )
            .await;

        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[
                SceneGraphEvent::Ime(crate::window::ImeEvent::Preedit {
                    text: "かな".into(),
                    cursor: Some((3, 6)),
                }),
                SceneGraphEvent::Clipboard(crate::window::ClipboardEvent::Paste("pasted".into())),
            ]
        );
        assert!(!WindowEvent::Ime(crate::window::ImeEvent::Enabled).skip_if_render_pending());
        assert!(
            !WindowEvent::Clipboard(crate::window::ClipboardEvent::Copy).skip_if_render_pending()
        );
    }

    #[tokio::test]
    async fn event_stream_debounce_commits_only_latest_event_on_matching_wake() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::CanvasResize],
                debounce: Some(DebounceConfig::new(20)),
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );

        let start = Instant::now();
        let first = manager
            .dispatch_event(
                &WindowEvent::CanvasResize(CanvasResizeEvent { size: [1.0, 1.0] }),
                &empty_rtree(),
                start,
            )
            .await;
        let second = manager
            .dispatch_event(
                &WindowEvent::CanvasResize(CanvasResizeEvent { size: [2.0, 2.0] }),
                &empty_rtree(),
                start + Duration::from_millis(5),
            )
            .await;
        assert!(events.lock().unwrap().is_empty());

        let request = |status: &UpdateStatus| {
            let [RuntimeHostCommand::RequestWakeup {
                key,
                deadline,
                generation,
            }] = status.commands.as_slice()
            else {
                panic!("expected one wakeup request")
            };
            (key.clone(), *deadline, *generation)
        };
        let (first_key, _, first_generation) = request(&first);
        let (key, deadline, generation) = request(&second);
        assert_eq!(first_key, key);
        assert!(generation > first_generation);

        manager
            .dispatch_event(
                &WindowEvent::RuntimeWake(RuntimeWakeEvent {
                    key: key.clone(),
                    generation: first_generation,
                }),
                &empty_rtree(),
                deadline,
            )
            .await;
        assert!(events.lock().unwrap().is_empty());

        let ready = manager
            .dispatch_event(
                &WindowEvent::RuntimeWake(RuntimeWakeEvent { key, generation }),
                &empty_rtree(),
                deadline,
            )
            .await;
        assert!(ready.rerender);
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                size: [2.0, 2.0],
            })]
        );
    }

    #[tokio::test]
    async fn between_stream_context_includes_start_and_previous_events() {
        let state = TestState::default();
        let events = state.events.clone();
        let contexts = state.contexts.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(drag_stream_config(), Arc::new(ContextRecordingHandler));

        let start = Instant::now();
        dispatch_cursor(&mut manager, [10.0, 20.0], start).await;
        dispatch_left_mouse(&mut manager, ElementState::Pressed, start).await;
        dispatch_cursor(&mut manager, [15.0, 22.0], start + Duration::from_millis(1)).await;
        dispatch_cursor(&mut manager, [18.0, 24.0], start + Duration::from_millis(2)).await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Released,
            start + Duration::from_millis(3),
        )
        .await;
        dispatch_cursor(&mut manager, [20.0, 26.0], start + Duration::from_millis(4)).await;

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].position(), Some([15.0, 22.0]));
        assert_eq!(events[1].position(), Some([18.0, 24.0]));

        let contexts = contexts.lock().unwrap();
        assert_eq!(contexts.len(), 2);

        let first_start = contexts[0].start_event.as_ref().expect("start event");
        assert_eq!(first_start.event.position(), Some([10.0, 20.0]));
        assert!(contexts[0].previous_event.is_none());

        let second_start = contexts[1].start_event.as_ref().expect("start event");
        assert_eq!(second_start.event.position(), Some([10.0, 20.0]));
        let previous = contexts[1].previous_event.as_ref().expect("previous event");
        assert_eq!(previous.event.position(), Some([15.0, 22.0]));
    }

    #[tokio::test]
    async fn between_stream_can_emit_matching_end_event_with_start_context() {
        let state = TestState::default();
        let events = state.events.clone();
        let contexts = state.contexts.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            drag_end_emit_stream_config(),
            Arc::new(ContextRecordingHandler),
        );

        let start = Instant::now();
        dispatch_cursor(&mut manager, [10.0, 20.0], start).await;
        dispatch_left_mouse(&mut manager, ElementState::Pressed, start).await;
        dispatch_cursor(&mut manager, [15.0, 25.0], start + Duration::from_millis(1)).await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Released,
            start + Duration::from_millis(2),
        )
        .await;

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SceneGraphEvent::MouseUp(_)));
        assert_eq!(events[0].position(), Some([15.0, 25.0]));

        let contexts = contexts.lock().unwrap();
        assert_eq!(contexts.len(), 1);
        let start_event = contexts[0].start_event.as_ref().expect("start event");
        assert_eq!(start_event.event.position(), Some([10.0, 20.0]));
    }

    #[tokio::test]
    async fn throttled_trigger_does_not_update_previous_event() {
        let state = TestState::default();
        let events = state.events.clone();
        let contexts = state.contexts.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                throttle: Some(10),
                ..drag_stream_config()
            },
            Arc::new(ContextRecordingHandler),
        );

        let start = Instant::now();
        dispatch_cursor(&mut manager, [0.0, 0.0], start).await;
        dispatch_left_mouse(&mut manager, ElementState::Pressed, start).await;
        dispatch_cursor(&mut manager, [1.0, 0.0], start + Duration::from_millis(1)).await;
        dispatch_cursor(&mut manager, [2.0, 0.0], start + Duration::from_millis(2)).await;
        dispatch_cursor(&mut manager, [3.0, 0.0], start + Duration::from_millis(12)).await;

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].position(), Some([1.0, 0.0]));
        assert_eq!(events[1].position(), Some([3.0, 0.0]));

        let contexts = contexts.lock().unwrap();
        let previous = contexts[1].previous_event.as_ref().expect("previous event");
        assert_eq!(previous.event.position(), Some([1.0, 0.0]));
    }

    #[tokio::test]
    async fn trigger_throttle_does_not_block_end_transition() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                throttle: Some(100),
                ..drag_stream_config()
            },
            Arc::new(ContextRecordingHandler),
        );

        let start = Instant::now();
        dispatch_cursor(&mut manager, [0.0, 0.0], start).await;
        dispatch_left_mouse(&mut manager, ElementState::Pressed, start).await;
        dispatch_cursor(&mut manager, [1.0, 0.0], start + Duration::from_millis(1)).await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Released,
            start + Duration::from_millis(2),
        )
        .await;
        dispatch_cursor(&mut manager, [2.0, 0.0], start + Duration::from_millis(3)).await;

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].position(), Some([1.0, 0.0]));
    }

    #[tokio::test]
    async fn context_filter_can_inspect_start_event() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                filter: Some(vec![EventStreamFilter::context(
                    |event, context, _rtree| {
                        let Some(start_x) = context
                            .start_event
                            .as_ref()
                            .and_then(|e| e.event.position().map(|position| position[0]))
                        else {
                            return false;
                        };
                        let Some(current_x) = event.position().map(|position| position[0]) else {
                            return false;
                        };
                        current_x > start_x
                    },
                )]),
                ..drag_stream_config()
            },
            Arc::new(ContextRecordingHandler),
        );

        let start = Instant::now();
        dispatch_cursor(&mut manager, [10.0, 0.0], start).await;
        dispatch_left_mouse(&mut manager, ElementState::Pressed, start).await;
        dispatch_cursor(&mut manager, [8.0, 0.0], start + Duration::from_millis(1)).await;
        dispatch_cursor(&mut manager, [12.0, 0.0], start + Duration::from_millis(2)).await;

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].position(), Some([12.0, 0.0]));
    }

    #[tokio::test]
    async fn start_event_does_not_consume_other_streams() {
        let state = TestState::default();
        let labels = state.labels.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                consume: true,
                ..drag_stream_config()
            },
            Arc::new(HandlerId("drag")),
        );
        manager.register_handler(left_mouse_down_config(), Arc::new(HandlerId("down")));

        let start = Instant::now();
        dispatch_cursor(&mut manager, [10.0, 20.0], start).await;
        dispatch_left_mouse(&mut manager, ElementState::Pressed, start).await;

        assert_eq!(
            labels.lock().unwrap().as_slice(),
            &["down:MouseDown".to_string()]
        );
    }

    #[tokio::test]
    async fn spaced_left_clicks_do_not_emit_double_click() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::Click],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::DoubleClick],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );

        let start = Instant::now();
        dispatch_cursor(&mut manager, [10.0, 20.0], start).await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Pressed,
            start + Duration::from_millis(1),
        )
        .await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Released,
            start + Duration::from_millis(2),
        )
        .await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Pressed,
            start + Duration::from_millis(700),
        )
        .await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Released,
            start + Duration::from_millis(701),
        )
        .await;

        let events = events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, SceneGraphEvent::Click(_)))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, SceneGraphEvent::DoubleClick(_)))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn double_click_emits_second_click_and_double_click() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::Click],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::DoubleClick],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );

        let start = Instant::now();
        dispatch_cursor(&mut manager, [10.0, 20.0], start).await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Pressed,
            start + Duration::from_millis(1),
        )
        .await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Released,
            start + Duration::from_millis(2),
        )
        .await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Pressed,
            start + Duration::from_millis(100),
        )
        .await;
        dispatch_left_mouse(
            &mut manager,
            ElementState::Released,
            start + Duration::from_millis(101),
        )
        .await;

        let events = events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, SceneGraphEvent::Click(_)))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, SceneGraphEvent::DoubleClick(_)))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn close_clicks_on_different_targets_do_not_emit_double_click() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::Click],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );
        manager.register_handler(
            EventStreamConfig {
                types: vec![SceneGraphEventType::DoubleClick],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );

        let rtree = empty_rtree();
        let start = Instant::now();
        manager
            .check_double_click(
                [10.0, 20.0],
                Some(test_mark_instance("first", 0)),
                &rtree,
                start + Duration::from_millis(1),
            )
            .await;
        manager
            .check_double_click(
                [11.0, 20.0],
                Some(test_mark_instance("second", 1)),
                &rtree,
                start + Duration::from_millis(100),
            )
            .await;

        let events = events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, SceneGraphEvent::Click(_)))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, SceneGraphEvent::DoubleClick(_)))
                .count(),
            0
        );
    }
    #[tokio::test]
    async fn keyboard_without_pointer_preserves_repeat_and_optional_position() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![
                    SceneGraphEventType::KeyPress,
                    SceneGraphEventType::KeyRelease,
                ],
                ..Default::default()
            },
            Arc::new(RecordingHandler),
        );
        for state in [ElementState::Pressed, ElementState::Released] {
            manager
                .dispatch_event(
                    &WindowEvent::KeyboardInput(WindowKeyboardInput {
                        key: Key::Named(NamedKey::Tab),
                        text: None,
                        repeat: true,
                        state,
                    }),
                    &empty_rtree(),
                    Instant::now(),
                )
                .await;
        }
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], SceneGraphEvent::KeyPress(e) if e.position.is_none() && e.mark_instance.is_none() && e.repeat)
        );
        assert!(matches!(&events[1], SceneGraphEvent::KeyRelease(e) if e.position.is_none()));
    }

    struct GestureOwner;
    #[async_trait]
    impl EventStreamHandler<TestState> for GestureOwner {
        async fn handle(
            &self,
            event: &SceneGraphEvent,
            state: &mut TestState,
            _: &SceneGraphRTree,
        ) -> UpdateStatus {
            state.events.lock().unwrap().push(event.clone());
            UpdateStatus {
                suppress_click: matches!(event, SceneGraphEvent::MouseDown(_)),
                consume: true,
                ..Default::default()
            }
        }
    }

    #[tokio::test]
    async fn owned_press_does_not_synthesize_clicks_or_double_clicks() {
        let state = TestState::default();
        let events = state.events.clone();
        let mut manager = EventStreamManager::new(state);
        manager.register_handler(
            EventStreamConfig {
                types: vec![
                    SceneGraphEventType::MouseDown,
                    SceneGraphEventType::MouseUp,
                    SceneGraphEventType::Click,
                    SceneGraphEventType::DoubleClick,
                ],
                ..Default::default()
            },
            Arc::new(GestureOwner),
        );
        let tree = empty_rtree();
        let now = Instant::now();
        manager
            .dispatch_event(
                &WindowEvent::CursorMoved(WindowCursorMoved { position: [1.0; 2] }),
                &tree,
                now,
            )
            .await;
        for _ in 0..2 {
            for state in [ElementState::Pressed, ElementState::Released] {
                manager
                    .dispatch_event(
                        &WindowEvent::MouseInput(WindowMouseInput {
                            state,
                            button: MouseButton::Left,
                        }),
                        &tree,
                        now,
                    )
                    .await;
            }
        }
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 4);
        assert!(events.iter().all(|e| matches!(
            e,
            SceneGraphEvent::MouseDown(_) | SceneGraphEvent::MouseUp(_)
        )));
    }
}
