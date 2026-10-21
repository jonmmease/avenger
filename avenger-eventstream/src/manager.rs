use crate::scene::{
    ModifiersState, SceneClickEvent, SceneCursorMovedEvent, SceneDoubleClickEvent,
    SceneFileChangedEvent, SceneGraphEvent, SceneGraphEventType, SceneKeyPressEvent,
    SceneKeyReleaseEvent, SceneMouseDownEvent, SceneMouseEnterEvent, SceneMouseLeaveEvent,
    SceneMouseUpEvent, SceneMouseWheelEvent,
};
use crate::stream::{EventStream, EventStreamConfig, UpdateStatus};
use crate::window::{ElementState, Key, MouseButton, NamedKey, WindowEvent, WindowKeyboardInput};
use async_trait::async_trait;
use avenger_common::time::{Duration, Instant};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;
use std::path::PathBuf;
use std::sync::Arc;

#[async_trait]
pub trait EventStreamHandler<State: Clone + Send + Sync + 'static> {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus;
}

#[derive(Clone)]
pub struct EventStreamManager<State: Clone + Send + Sync + 'static> {
    state: State,
    /// The streams, in the order that they hear events: higher layers first, and within a
    /// layer, the order that they were added.
    streams: Vec<EventStream<State>>,
    current_mark: Option<MarkInstance>,
    last_click: Option<(Instant, [f32; 2])>,
    /// The last press: when and where, its button, and how many presses in a row it made.
    last_press: Option<(Instant, [f32; 2], MouseButton, u32)>,
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
            last_press: None,
            double_click_threshold: Duration::from_millis(500),
            double_click_distance: 5.0,
            current_cursor_position: None,
            mousedown_mark: None,
            mousedown_button: None,
            modifiers: ModifiersState::default(),
        }
    }

    /// Register a new event handler with the given configuration, in the scene's layer, 0.
    pub fn register_handler(
        &mut self,
        config: EventStreamConfig,
        handler: Arc<dyn EventStreamHandler<State>>,
    ) {
        self.register_handler_in_layer(0, config, handler);
    }

    /// Register a new event handler with the given configuration in `layer`, such as an
    /// overlay's over the scene's. The streams in higher layers hear each event first, and within
    /// a layer, the streams added first hear first, so a stream that consumes an event takes it
    /// from the streams beneath it.
    pub fn register_handler_in_layer(
        &mut self,
        layer: usize,
        config: EventStreamConfig,
        handler: Arc<dyn EventStreamHandler<State>>,
    ) {
        let mut stream = EventStream::new(config, handler);
        stream.layer = layer;
        let at = (self.streams.iter())
            .position(|stream| stream.layer < layer)
            .unwrap_or(self.streams.len());
        self.streams.insert(at, stream);
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

        let mut update_status = UpdateStatus::UNCHANGED;

        // Convert window event to scene graph event
        let scene_event = match event {
            WindowEvent::MouseInput(input) => {
                if let Some(position) = self.current_cursor_position {
                    let mark_instance = self.get_mark_path_for_event_at_position(&position, rtree);

                    if input.state == ElementState::Pressed {
                        // Store both mark and button
                        self.mousedown_mark = mark_instance.clone();
                        self.mousedown_button = Some(input.button);
                        let count = self.press_count(position, input.button, instant);
                        Some(SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                            position,
                            button: input.button,
                            mark_instance,
                            modifiers: self.modifiers,
                            count,
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
            WindowEvent::WindowMoved(e) => Some(SceneGraphEvent::WindowMoved(e.clone())),
            WindowEvent::WindowFocused(focused) => Some(SceneGraphEvent::WindowFocused(*focused)),
            WindowEvent::WindowCloseRequested => Some(SceneGraphEvent::WindowCloseRequested),
            WindowEvent::CursorLeft => Some(SceneGraphEvent::CursorLeft),
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

    /// Dispatch [`SceneGraphEvent::TreeRebuilt`] to its streams, after the app rebuilds the
    /// hit-testing tree.
    pub async fn dispatch_tree_rebuilt(
        &mut self,
        rtree: &SceneGraphRTree,
        instant: Instant,
    ) -> UpdateStatus {
        self.dispatch_single_event(&SceneGraphEvent::TreeRebuilt, rtree, instant, None)
            .await
    }

    /// When the earliest wake is due: the earliest time that a debounced stream's burst settles,
    /// for the host's timer. Dispatch the wakes then with
    /// [`EventStreamManager::dispatch_due_wakes`].
    pub fn next_wake(&self) -> Option<Instant> {
        self.streams
            .iter()
            .filter_map(EventStream::settles_at)
            .min()
    }

    /// Run the wakes that are due at `now`, earliest first: each debounced stream whose burst
    /// settled handles the burst's last event.
    pub async fn dispatch_due_wakes(
        &mut self,
        rtree: &SceneGraphRTree,
        now: Instant,
    ) -> UpdateStatus {
        let mut due: Vec<(Instant, usize)> = self
            .streams
            .iter()
            .enumerate()
            .filter_map(|(index, stream)| Some((stream.settles_at()?, index)))
            .filter(|(settles_at, _)| *settles_at <= now)
            .collect();
        due.sort();
        let mut update_status = UpdateStatus::UNCHANGED;
        for (_, index) in due {
            let stream = &mut self.streams[index];
            if let Some(event) = stream.settle() {
                stream.last_handled_time = Some(now);
                update_status = update_status
                    .merge(&stream.handler.handle(&event, &mut self.state, rtree).await);
            }
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

        let mut update_status = UpdateStatus::UNCHANGED;

        for stream in &mut self.streams {
            if stream.matches_and_update(event, mark_instance.as_ref(), instant) {
                // A debounced stream takes the event into its burst, and handles it later
                // unless it's a burst's first event that the stream handles at once.
                if stream.debounce(event, instant).unwrap_or(true) {
                    // Update last handled time
                    stream.last_handled_time = Some(instant);

                    // Call handler and merge update status
                    update_status = update_status
                        .merge(&stream.handler.handle(event, &mut self.state, rtree).await);
                }

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

        let mut update_status = UpdateStatus::UNCHANGED;
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

    /// How many presses of `button` in a row this press makes: one more than the last press's
    /// count when it came within the double click's time and distance, or else 1.
    fn press_count(&mut self, position: [f32; 2], button: MouseButton, instant: Instant) -> u32 {
        let count = match self.last_press {
            Some((time, at, pressed, count))
                if pressed == button
                    && instant.duration_since(time) <= self.double_click_threshold
                    && (position[0] - at[0]).hypot(position[1] - at[1])
                        <= self.double_click_distance =>
            {
                count + 1
            }
            _ => 1,
        };
        self.last_press = Some((instant, position, button, count));
        count
    }

    async fn check_double_click(
        &mut self,
        position: [f32; 2],
        mark_instance: Option<MarkInstance>,
        rtree: &SceneGraphRTree,
        instant: Instant,
    ) -> UpdateStatus {
        let mut update_status = UpdateStatus::UNCHANGED;

        if let Some((last_time, last_pos)) = &self.last_click {
            let time_diff = instant.duration_since(*last_time);
            let distance =
                ((position[0] - last_pos[0]).powi(2) + (position[1] - last_pos[1]).powi(2)).sqrt();

            if time_diff <= self.double_click_threshold && distance <= self.double_click_distance {
                // Double click detected - dispatch event
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
                // Reset last click
                self.last_click = None;
                return update_status;
            }
        }

        // Not a double click, emit single left-click and store for potential future double-click
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
        self.last_click = Some((instant, position));

        update_status
    }

    pub fn modifiers(&self) -> ModifiersState {
        self.modifiers
    }
}
