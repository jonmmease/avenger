use std::sync::Arc;

use async_trait::async_trait;
use avenger_common::time::{Duration, Instant};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;

use crate::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent, SceneGraphEventType},
};

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

#[derive(Clone, Debug)]
pub struct EventStreamEventSnapshot {
    pub event: SceneGraphEvent,
    pub mark_instance: Option<MarkInstance>,
    pub instant: Instant,
}

impl EventStreamEventSnapshot {
    fn new(
        event: &SceneGraphEvent,
        mark_instance: Option<&MarkInstance>,
        instant: Instant,
    ) -> Self {
        Self {
            event: event.clone(),
            mark_instance: mark_instance.cloned(),
            instant,
        }
    }
}

#[derive(Clone, Default, Debug)]
pub struct EventStreamContext {
    pub mark_instance: Option<MarkInstance>,
    pub start_event: Option<EventStreamEventSnapshot>,
    pub previous_event: Option<EventStreamEventSnapshot>,
}

impl EventStreamContext {
    fn new(
        mark_instance: Option<&MarkInstance>,
        start_event: Option<EventStreamEventSnapshot>,
        previous_event: Option<EventStreamEventSnapshot>,
    ) -> Self {
        Self {
            mark_instance: mark_instance.cloned(),
            start_event,
            previous_event,
        }
    }
}

type EventStreamFilterFn =
    dyn Fn(&SceneGraphEvent, &EventStreamContext, &SceneGraphRTree) -> bool + Send + Sync + 'static;

/// Wrapper around a filter function that supports Debug formatting
#[derive(Clone)]
pub struct EventStreamFilter {
    predicate: Arc<EventStreamFilterFn>,
}

impl EventStreamFilter {
    pub fn event(f: impl Fn(&SceneGraphEvent) -> bool + Send + Sync + 'static) -> Self {
        Self {
            predicate: Arc::new(move |event, _context, _rtree| f(event)),
        }
    }

    pub fn context(
        f: impl Fn(&SceneGraphEvent, &EventStreamContext, &SceneGraphRTree) -> bool
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            predicate: Arc::new(f),
        }
    }

    pub fn matches(
        &self,
        event: &SceneGraphEvent,
        context: &EventStreamContext,
        rtree: &SceneGraphRTree,
    ) -> bool {
        (self.predicate)(event, context, rtree)
    }
}

impl std::fmt::Debug for EventStreamFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventStreamFilter")
    }
}

#[derive(Clone, Default, Debug)]
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
    pub filter: Option<Vec<EventStreamFilter>>,

    /// If specified, only events that occur after the start stream has been triggered
    /// and before the end stream has been triggered will be included
    pub between: Option<(Box<EventStreamConfig>, Box<EventStreamConfig>)>,

    /// If specified, only events associated with the specified mark paths will be included
    pub mark_paths: Option<Vec<Vec<usize>>>,

    /// Minimum time (in milliseconds) between events
    pub throttle: Option<u64>,
}

#[derive(Clone, Default, Debug, Copy)]
pub struct UpdateStatus {
    pub rerender: bool,
    pub rebuild_geometry: bool,
}

impl UpdateStatus {
    pub fn merge(&self, other: &UpdateStatus) -> UpdateStatus {
        UpdateStatus {
            rerender: self.rerender || other.rerender,
            rebuild_geometry: self.rebuild_geometry || other.rebuild_geometry,
        }
    }
}

/// Internal struct representing the state of an event stream and it's handler
#[derive(Clone)]
pub(crate) struct EventStream<State: Clone + Send + Sync + 'static> {
    pub(crate) config: EventStreamConfig,
    pub(crate) between_state: Option<BetweenState>,
    pub(crate) last_handled_time: Option<Instant>,
    pub(crate) previous_event: Option<EventStreamEventSnapshot>,
    pub(crate) handler: Arc<dyn EventStreamHandler<State>>,
}

#[derive(Clone)]
pub(crate) struct BetweenState {
    start_event: Option<EventStreamEventSnapshot>,
    start_stream: Box<EventStream<()>>,
    end_stream: Box<EventStream<()>>,
}

// handler that does nothing
struct NoopHandler;

#[async_trait]
impl EventStreamHandler<()> for NoopHandler {
    async fn handle(&self, _: &SceneGraphEvent, _: &mut (), _: &SceneGraphRTree) -> UpdateStatus {
        Default::default()
    }
}

impl<State: Clone + Send + Sync + 'static> EventStream<State> {
    pub(crate) fn new(
        config: EventStreamConfig,
        handler: Arc<dyn EventStreamHandler<State>>,
    ) -> Self {
        // Initialize between_state if config.between is specified
        let between_state = config
            .between
            .as_ref()
            .map(|(start_cfg, end_cfg)| BetweenState {
                start_event: None,
                start_stream: Box::new(EventStream::new(
                    start_cfg.as_ref().clone(),
                    Arc::new(NoopHandler),
                )),
                end_stream: Box::new(EventStream::new(
                    end_cfg.as_ref().clone(),
                    Arc::new(NoopHandler),
                )),
            });

        Self {
            config,
            between_state,
            last_handled_time: None,
            previous_event: None,
            handler,
        }
    }

    pub(crate) fn matches_and_update(
        &mut self,
        event: &SceneGraphEvent,
        mark_instance: Option<&MarkInstance>,
        rtree: &SceneGraphRTree,
        now: Instant,
    ) -> Option<EventStreamContext> {
        let current_snapshot = EventStreamEventSnapshot::new(event, mark_instance, now);
        let previous_event = self.previous_event.clone();
        let mut start_event = self
            .between_state
            .as_ref()
            .and_then(|between| between.start_event.clone());

        // Handle between state
        if let Some(between) = &mut self.between_state {
            if between.start_event.is_none() {
                // Not started yet, check if this is start event
                let context = EventStreamContext::new(mark_instance, None, previous_event.clone());
                if between.start_stream.matches_event(event, &context, rtree) {
                    between.start_event = Some(current_snapshot);
                }
                return None;
            } else {
                // Started, check if this is end event
                start_event = between.start_event.clone();
                let context = EventStreamContext::new(
                    mark_instance,
                    start_event.clone(),
                    previous_event.clone(),
                );
                if between.end_stream.matches_event(event, &context, rtree) {
                    between.start_event = None;
                    return None;
                }
            }
        }

        let context = EventStreamContext::new(mark_instance, start_event, previous_event);

        // Check if event matches and throttling allows it
        if self.matches_event(event, &context, rtree) && self.should_handle_event(now) {
            Some(context)
        } else {
            None
        }
    }

    pub(crate) fn mark_accepted(
        &mut self,
        event: &SceneGraphEvent,
        mark_instance: Option<&MarkInstance>,
        now: Instant,
    ) {
        self.last_handled_time = Some(now);
        self.previous_event = Some(EventStreamEventSnapshot::new(event, mark_instance, now));
    }

    pub(crate) fn matches_event(
        &self,
        event: &SceneGraphEvent,
        context: &EventStreamContext,
        rtree: &SceneGraphRTree,
    ) -> bool {
        // Check event type matches
        if !self.config.types.contains(&event.event_type()) {
            return false;
        }

        // Apply filters
        if let Some(filters) = &self.config.filter {
            for filter in filters {
                if !filter.matches(event, context, rtree) {
                    return false;
                }
            }
        }

        // Check source group if specified
        if let Some(group) = &self.config.source_group {
            if let Some(mark_instance) = &context.mark_instance {
                if mark_instance.mark_path.len() < group.len()
                    || group != &mark_instance.mark_path[0..group.len()]
                {
                    // Mark path is not under the source group, so ignore
                    return false;
                }
            }
        }

        // Check mark paths are specified
        if let Some(paths) = &self.config.mark_paths {
            if let Some(mark_instance) = &context.mark_instance {
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
                    tracing::trace!(
                        target: "avenger_eventstream::resize",
                        throttle_ms = throttle,
                        "eventstream throttle drop"
                    );
                    return false;
                }
            }
        }
        true
    }
}
