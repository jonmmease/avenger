use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use avenger_common::time::{Duration, Instant};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;

use crate::{
    scene::{
        ModifiersState, SceneClickEvent, SceneCursorMovedEvent, SceneDoubleClickEvent,
        SceneFileChangedEvent, SceneGraphEvent, SceneGraphEventType, SceneKeyPressEvent,
        SceneKeyReleaseEvent, SceneMouseDownEvent, SceneMouseEnterEvent, SceneMouseLeaveEvent,
        SceneMouseUpEvent, SceneMouseWheelEvent,
    },
    stream::{EventStream, EventStreamConfig, EventStreamContext, UpdateStatus},
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
            modifiers: ModifiersState::default(),
        }
    }

    /// Register a new event handler with the given configuration
    pub fn register_handler(
        &mut self,
        config: EventStreamConfig,
        handler: Arc<dyn EventStreamHandler<State>>,
    ) {
        let stream = EventStream::new(config, handler);
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
        if let WindowEvent::KeyboardInput(input) = event {
            self.update_modifiers(input);
        }

        // Update cursor position tracking
        if let Some(position) = event.position() {
            self.current_cursor_position = Some(position);
        }

        let mut update_status = UpdateStatus::default();

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
                        if self.mousedown_mark.as_ref() == mark_instance.as_ref()
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
                if let Some(position) = self.current_cursor_position {
                    let mark_instance = self.get_mark_path_for_event_at_position(&position, rtree);
                    if e.state == ElementState::Pressed {
                        Some(SceneGraphEvent::KeyPress(SceneKeyPressEvent {
                            position,
                            key: e.key,
                            text: e.text.clone(),
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
                } else {
                    None
                }
            }
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

        update_status
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

        for stream in &mut self.streams {
            if let Some(context) =
                stream.matches_and_update(event, mark_instance.as_ref(), rtree, instant)
            {
                // Call handler and merge update status
                update_status = update_status.merge(
                    &stream
                        .handler
                        .handle_with_context(event, &context, &mut self.state, rtree)
                        .await,
                );

                stream.mark_accepted(event, mark_instance.as_ref(), instant);

                // Handle consume flag
                if stream.config.consume {
                    break;
                }
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

    use avenger_scenegraph::scene_graph::SceneGraph;

    use super::*;
    use crate::{
        stream::{EventStreamConfig, EventStreamFilter},
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

    #[derive(Clone)]
    struct HandlerId(&'static str);

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
}
