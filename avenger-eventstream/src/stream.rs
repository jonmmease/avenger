use crate::scene::{
    SceneClickEvent, SceneCursorMovedEvent, SceneDoubleClickEvent, SceneGraphEvent,
    SceneGraphEventType, SceneKeyPressEvent, SceneKeyReleaseEvent, SceneMouseDownEvent,
    SceneMouseEnterEvent, SceneMouseLeaveEvent, SceneMouseUpEvent, SceneMouseWheelEvent,
};
use crate::window::{ElementState, MouseButton, WindowEvent};
use crate::{Key, NamedKey, WindowKeyboardInput};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct DebounceConfig {
    /// The number of milliseconds to delay
    pub wait: u64,
    /// The maximum time func is allowed to be delayed before it's invoked
    pub max_wait: Option<u64>,
    /// Specify invoking on the leading edge of the timeout
    pub leading: bool,
}

impl DebounceConfig {
    pub fn new(wait: u64) -> Self {
        Self {
            wait,
            leading: false,
            max_wait: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct EventStreamConfig {
    /// Event types to include in the stream
    pub types: Vec<SceneGraphEventType>,

    /// If specified, only events associated with marks within
    /// the specified scene group will be included
    pub source_group: Option<Vec<usize>>,

    /// If true, the event will be consumed by the event stream and
    /// not propagated to other streams
    pub consume: bool,

    /// If specified, only events matching all of the filters will be included
    pub filter: Option<Vec<Arc<dyn Fn(&SceneGraphEvent) -> bool>>>,

    /// If specified, only events that occur after the start stream has been triggered
    /// and before the end stream has been triggered will be included
    pub between: Option<(Box<EventStreamConfig>, Box<EventStreamConfig>)>,

    /// If specified, only events associated with the specified mark paths will be included
    pub mark_paths: Option<Vec<Vec<usize>>>,

    /// Minimum time (in milliseconds) between events
    pub throttle: Option<u64>,
}

/// Internal struct representing the state of an event stream and it's handler
#[derive(Clone)]
struct EventStream {
    config: EventStreamConfig,
    between_state: Option<BetweenState>,
    last_handled_time: Option<Instant>,
    handler: Arc<dyn Fn(&SceneGraphEvent)>,
}

#[derive(Clone)]
struct BetweenState {
    started: bool,
    start_stream: Box<EventStream>,
    end_stream: Box<EventStream>,
}

impl EventStream {
    fn new(config: EventStreamConfig, handler: Arc<dyn Fn(&SceneGraphEvent)>) -> Self {
        // Initialize between_state if config.between is specified
        let between_state = config
            .between
            .as_ref()
            .map(|(start_cfg, end_cfg)| BetweenState {
                started: false,
                start_stream: Box::new(EventStream::new(
                    start_cfg.as_ref().clone(),
                    handler.clone(),
                )),
                end_stream: Box::new(EventStream::new(end_cfg.as_ref().clone(), handler.clone())),
            });

        Self {
            config,
            between_state,
            last_handled_time: None,
            handler,
        }
    }

    fn matches_and_update(
        &mut self,
        event: &SceneGraphEvent,
        mark_instance: Option<&MarkInstance>,
        now: Instant,
    ) -> bool {
        // Handle between state
        if let Some(between) = &mut self.between_state {
            if !between.started {
                // Not started yet, check if this is start event
                if between.start_stream.matches_event(event, mark_instance) {
                    between.started = true;
                }
                return false;
            } else {
                // Started, check if this is end event
                if between.end_stream.matches_event(event, mark_instance) {
                    between.started = false;
                    return false;
                }
            }
        }

        // Check if event matches and throttling allows it
        if self.matches_event(event, mark_instance) && self.should_handle_event(now) {
            true
        } else {
            false
        }
    }

    fn matches_event(&self, event: &SceneGraphEvent, mark_instance: Option<&MarkInstance>) -> bool {
        // Check event type matches
        if !self.config.types.contains(&event.event_type()) {
            return false;
        }

        // Apply filters
        if let Some(filters) = &self.config.filter {
            for filter in filters {
                if !filter(event) {
                    return false;
                }
            }
        }

        // Check source group if specified
        if let Some(group) = &self.config.source_group {
            if let Some(mark_instance) = mark_instance {
                if group != &mark_instance.mark_path[0..group.len()] {
                    // Mark path is not under the source group, so ignore
                    return false;
                }
            }
        }

        // Check mark paths are specified
        if let Some(paths) = &self.config.mark_paths {
            if let Some(mark_instance) = mark_instance {
                if !paths.contains(&mark_instance.mark_path) {
                    return false;
                }
            } else {
                return false;
            }
        }

        true
    }

    fn should_handle_event(&mut self, now: Instant) -> bool {
        if let Some(throttle) = self.config.throttle {
            if let Some(last_time) = self.last_handled_time {
                if now.duration_since(last_time) < Duration::from_millis(throttle) {
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModifiersState {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

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
