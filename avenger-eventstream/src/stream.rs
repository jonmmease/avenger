use crate::scene::{
    SceneClickEvent, SceneCursorMovedEvent, SceneDoubleClickEvent, SceneGraphEvent,
    SceneGraphEventType, SceneKeyPressEvent, SceneKeyReleaseEvent, SceneMouseDownEvent,
    SceneMouseEnterEvent, SceneMouseLeaveEvent, SceneMouseUpEvent, SceneMouseWheelEvent,
};
use crate::window::{ElementState, MouseButton, WindowEvent};
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
pub(crate) struct EventStream {
    pub(crate) config: EventStreamConfig,
    pub(crate) between_state: Option<BetweenState>,
    pub(crate) last_handled_time: Option<Instant>,
    pub(crate) handler: Arc<dyn Fn(&SceneGraphEvent)>,
}

#[derive(Clone)]
struct BetweenState {
    started: bool,
    start_stream: Box<EventStream>,
    end_stream: Box<EventStream>,
}

impl EventStream {
    pub(crate) fn new(config: EventStreamConfig, handler: Arc<dyn Fn(&SceneGraphEvent)>) -> Self {
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

    pub(crate) fn matches_and_update(
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

    pub(crate) fn matches_event(
        &self,
        event: &SceneGraphEvent,
        mark_instance: Option<&MarkInstance>,
    ) -> bool {
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

    pub(crate) fn should_handle_event(&mut self, now: Instant) -> bool {
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
