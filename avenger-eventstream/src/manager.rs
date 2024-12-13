use crate::scene::{
    ModifiersState, SceneClickEvent, SceneCursorMovedEvent, SceneDoubleClickEvent, SceneGraphEvent,
    SceneKeyPressEvent, SceneKeyReleaseEvent, SceneMouseDownEvent, SceneMouseEnterEvent,
    SceneMouseLeaveEvent, SceneMouseUpEvent, SceneMouseWheelEvent,
};
use crate::stream::{EventStream, EventStreamConfig};
use crate::window::{ElementState, Key, MouseButton, NamedKey, WindowEvent, WindowKeyboardInput};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct EventStreamManager {
    streams: Vec<EventStream>,
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

impl EventStreamManager {
    pub fn new() -> Self {
        Self {
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
    pub fn register_handler<F>(&mut self, config: EventStreamConfig, handler: F)
    where
        F: Fn(&SceneGraphEvent) + 'static,
    {
        let stream = EventStream::new(config, Arc::new(handler));
        self.streams.push(stream);
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

    pub fn dispatch_event(
        &mut self,
        event: &WindowEvent,
        rtree: &SceneGraphRTree,
        instant: Instant,
    ) {
        // Update modifier state based on keyboard events
        if let WindowEvent::KeyboardInput(input) = event {
            self.update_modifiers(&input);
        }

        // Update cursor position tracking
        if let Some(position) = event.position() {
            self.current_cursor_position = Some(position);
        }

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
                                self.check_double_click(
                                    position,
                                    mark_instance.clone(),
                                    rtree,
                                    instant,
                                );
                            } else {
                                self.dispatch_single_event(
                                    &SceneGraphEvent::Click(SceneClickEvent {
                                        position,
                                        button: input.button.clone(),
                                        mark_instance: mark_instance.clone(),
                                        modifiers: self.modifiers,
                                    }),
                                    rtree,
                                    instant,
                                    None,
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
            self.handle_mark_mouse_events(position, rtree, instant);
        }

        // Dispatch the converted event if any
        if let Some(scene_event) = scene_event {
            self.dispatch_single_event(&scene_event, rtree, instant, None);
        }
    }

    fn dispatch_single_event(
        &mut self,
        event: &SceneGraphEvent,
        rtree: &SceneGraphRTree,
        instant: Instant,
        mark_instance: Option<MarkInstance>,
    ) {
        let mark_instance = mark_instance.or_else(|| self.get_mark_path_for_event(event, rtree));

        for stream in &mut self.streams {
            if stream.matches_and_update(event, mark_instance.as_ref(), instant) {
                // Update last handled time
                stream.last_handled_time = Some(instant);

                // Call handler
                (stream.handler)(event);

                // Handle consume flag
                if stream.config.consume {
                    break;
                }
            }
        }
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

    fn handle_mark_mouse_events(
        &mut self,
        position: [f32; 2],
        rtree: &SceneGraphRTree,
        instant: Instant,
    ) {
        let current_mark = self.get_mark_path_for_event_at_position(&position, rtree);

        // Handle mark enter/leave
        match (&self.current_mark, &current_mark) {
            (Some(prev), Some(curr)) if prev != curr => {
                // Mark changed - generate leave then enter
                // Use the previous mark instance for leave event
                self.dispatch_single_event(
                    &SceneGraphEvent::MouseLeave(SceneMouseLeaveEvent {
                        position,
                        mark_instance: prev.clone(),
                        modifiers: self.modifiers,
                    }),
                    rtree,
                    instant,
                    Some(prev.clone()),
                );
                // Use the current mark instance for enter event
                self.dispatch_single_event(
                    &SceneGraphEvent::MouseEnter(SceneMouseEnterEvent {
                        position,
                        mark_instance: curr.clone(),
                        modifiers: self.modifiers,
                    }),
                    rtree,
                    instant,
                    Some(curr.clone()),
                );
            }
            (Some(prev), None) => {
                // Left mark - generate leave
                self.dispatch_single_event(
                    &SceneGraphEvent::MouseLeave(SceneMouseLeaveEvent {
                        position,
                        mark_instance: prev.clone(),
                        modifiers: self.modifiers,
                    }),
                    rtree,
                    instant,
                    Some(prev.clone()),
                );
            }
            (None, Some(curr)) => {
                // Entered mark - generate enter
                self.dispatch_single_event(
                    &SceneGraphEvent::MouseEnter(SceneMouseEnterEvent {
                        position,
                        mark_instance: curr.clone(),
                        modifiers: self.modifiers,
                    }),
                    rtree,
                    instant,
                    Some(curr.clone()),
                );
            }
            _ => {}
        }

        // Update current mark
        self.current_mark = current_mark;
    }

    fn check_double_click(
        &mut self,
        position: [f32; 2],
        mark_instance: Option<MarkInstance>,
        rtree: &SceneGraphRTree,
        instant: Instant,
    ) {
        if let Some((last_time, last_pos)) = &self.last_click {
            let time_diff = instant.duration_since(*last_time);
            let distance =
                ((position[0] - last_pos[0]).powi(2) + (position[1] - last_pos[1]).powi(2)).sqrt();

            if time_diff <= self.double_click_threshold && distance <= self.double_click_distance {
                // Double click detected - dispatch event
                self.dispatch_single_event(
                    &SceneGraphEvent::DoubleClick(SceneDoubleClickEvent {
                        position,
                        mark_instance,
                        modifiers: self.modifiers,
                    }),
                    rtree,
                    instant,
                    None,
                );
                // Reset last click
                self.last_click = None;
                return;
            }
        }

        // Not a double click, emit single left-click and store for potential future double-click
        self.dispatch_single_event(
            &SceneGraphEvent::Click(SceneClickEvent {
                position,
                button: MouseButton::Left,
                mark_instance: mark_instance.clone(),
                modifiers: self.modifiers,
            }),
            rtree,
            instant,
            None,
        );
        self.last_click = Some((instant, position));
    }

    pub fn modifiers(&self) -> ModifiersState {
        self.modifiers
    }
}

impl Default for EventStreamManager {
    fn default() -> Self {
        Self::new()
    }
}
