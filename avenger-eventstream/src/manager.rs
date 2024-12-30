use crate::scene::{
    ModifiersState, SceneClickEvent, SceneCursorMovedEvent, SceneDoubleClickEvent, SceneGraphEvent,
    SceneKeyPressEvent, SceneKeyReleaseEvent, SceneMouseDownEvent, SceneMouseEnterEvent,
    SceneMouseLeaveEvent, SceneMouseUpEvent, SceneMouseWheelEvent,
};
use crate::stream::{EventStream, EventStreamConfig, UpdateStatus};
use crate::window::{ElementState, Key, MouseButton, NamedKey, WindowEvent, WindowKeyboardInput};
use async_trait::async_trait;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[async_trait]
pub trait EventStreamHandler<State: Clone + Send + Sync + 'static> {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus;
}

// impl<State, F> EventStreamHandler<State> for F
// where
//     State: Clone + Send + Sync + 'static,
//     F: Fn(&SceneGraphEvent, &mut State, &SceneGraphRTree) -> UpdateStatus + 'static,
// {
//     async fn handle(
//         &self,
//         event: &SceneGraphEvent,
//         state: &mut State,
//         rtree: &SceneGraphRTree,
//     ) -> UpdateStatus {
//         self(event, state, rtree)
//     }
// }

pub struct EventStreamManager<State: Clone + Send + Sync + 'static> {
    state: State,
    streams: Vec<EventStream<State>>,
    current_mark: Option<MarkInstance>,
    last_click: Option<(Instant, [f32; 2])>,
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

    pub async fn dispatch_event(
        &mut self,
        event: &WindowEvent,
        rtree: &SceneGraphRTree,
        instant: Instant,
    ) -> UpdateStatus {
        // Update modifier state based on keyboard events
        if let WindowEvent::KeyboardInput(input) = event {
            self.update_modifiers(&input);
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
                        self.mousedown_button = Some(input.button.clone());
                        Some(SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                            position,
                            button: input.button.clone(),
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
                                                button: input.button.clone(),
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
                            button: input.button.clone(),
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
                            key: e.key.clone(),
                            mark_instance,
                            modifiers: self.modifiers,
                        }))
                    } else {
                        Some(SceneGraphEvent::KeyRelease(SceneKeyReleaseEvent {
                            position,
                            key: e.key.clone(),
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
            if stream.matches_and_update(event, mark_instance.as_ref(), instant) {
                // Update last handled time
                stream.last_handled_time = Some(instant);

                // Call handler and merge update status
                update_status = update_status
                    .merge(&stream.handler.handle(event, &mut self.state, rtree).await);

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
