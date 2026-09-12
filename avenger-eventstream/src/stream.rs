use std::sync::Arc;

use async_trait::async_trait;
use avenger_common::{
    cursor::CursorStyle,
    time::{Duration, Instant},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;

use crate::{
    manager::EventStreamHandler,
    runtime::{
        DebouncedCommit, DebouncedCommitUpdate, RuntimeHostCommand, RuntimeWakeEvent,
        RuntimeWakeKey,
    },
    scene::{SceneGraphEvent, SceneGraphEventType},
};

pub use crate::runtime::DebounceConfig;

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
    pub current_event: Option<EventStreamEventSnapshot>,
    pub start_event: Option<EventStreamEventSnapshot>,
    pub previous_event: Option<EventStreamEventSnapshot>,
}

impl EventStreamContext {
    fn new(
        mark_instance: Option<&MarkInstance>,
        current_event: Option<EventStreamEventSnapshot>,
        start_event: Option<EventStreamEventSnapshot>,
        previous_event: Option<EventStreamEventSnapshot>,
    ) -> Self {
        Self {
            mark_instance: mark_instance.cloned(),
            current_event,
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

    /// If true, the matching between end event is emitted once with the frozen
    /// start event context before the between state is cleared.
    pub emit_between_end_event: bool,

    /// If specified, only events associated with the specified mark paths will be included
    pub mark_paths: Option<Vec<Vec<usize>>>,

    /// Public mark target names corresponding to `mark_paths`.
    ///
    /// Paths alone are insufficient once compiled marks are nested in independently
    /// rendered scene groups: two unrelated marks can have the same local path suffix.
    /// Requiring the stable public name keeps target identity owner-scoped.
    pub mark_names: Option<Vec<String>>,

    /// Minimum time (in milliseconds) between events
    pub throttle: Option<u64>,

    /// Debounce matching events through the host's exact wake-up scheduler.
    pub debounce: Option<DebounceConfig>,
}

#[derive(Clone, Default, Debug)]
pub struct UpdateStatus {
    pub rerender: bool,
    pub rebuild_geometry: bool,
    pub cursor: Option<CursorStyle>,
    pub commands: Vec<RuntimeHostCommand>,
    /// Stop propagation after this handler. Unlike `EventStreamConfig::consume`,
    /// this is decided dynamically from the accepted event.
    pub consume: bool,
    /// Suppress synthesized clicks for the current pointer press/release sequence.
    pub suppress_click: bool,
    /// Whether the handler adopted the matched event. `None` preserves the
    /// historical behavior for non-transactional handlers and is interpreted
    /// as committed by the manager.
    pub admission: Option<EventAdmission>,
}

/// The adoption result of a matched event after its handler has run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventAdmission {
    Rejected,
    Committed,
    Failed,
}

impl EventAdmission {
    pub(crate) fn merge(self, other: Self) -> Self {
        use EventAdmission::{Committed, Failed, Rejected};
        match (self, other) {
            (Failed, _) | (_, Failed) => Failed,
            (Committed, _) | (_, Committed) => Committed,
            (Rejected, Rejected) => Rejected,
        }
    }
}

impl UpdateStatus {
    pub fn merge(&self, other: &UpdateStatus) -> UpdateStatus {
        UpdateStatus {
            rerender: self.rerender || other.rerender,
            rebuild_geometry: self.rebuild_geometry || other.rebuild_geometry,
            cursor: other.cursor.or(self.cursor),
            commands: self
                .commands
                .iter()
                .chain(&other.commands)
                .cloned()
                .collect(),
            consume: self.consume || other.consume,
            suppress_click: self.suppress_click || other.suppress_click,
            admission: match (self.admission, other.admission) {
                (Some(left), Some(right)) => Some(left.merge(right)),
                (Some(admission), None) | (None, Some(admission)) => Some(admission),
                (None, None) => None,
            },
        }
    }
}

#[derive(Clone)]
pub(crate) struct EventStreamAdmissionCheckpoint {
    between_state: Option<BetweenState>,
    last_handled_time: Option<Instant>,
    previous_event: Option<EventStreamEventSnapshot>,
}

/// Internal struct representing the state of an event stream and it's handler
#[derive(Clone)]
pub(crate) struct EventStream<State: Clone + Send + Sync + 'static> {
    pub(crate) config: EventStreamConfig,
    pub(crate) between_state: Option<BetweenState>,
    pub(crate) last_handled_time: Option<Instant>,
    pub(crate) previous_event: Option<EventStreamEventSnapshot>,
    pub(crate) handler: Arc<dyn EventStreamHandler<State>>,
    debounce: Option<DebouncedCommit<DebouncedEvent>>,
    wake_key: RuntimeWakeKey,
}

#[derive(Clone)]
pub(crate) struct DebouncedEvent {
    pub(crate) event: SceneGraphEvent,
    pub(crate) context: EventStreamContext,
    pub(crate) checkpoint: EventStreamAdmissionCheckpoint,
}

#[derive(Clone)]
pub(crate) struct BetweenState {
    start_event: Option<EventStreamEventSnapshot>,
    start_stream: Box<EventStream<()>>,
    end_stream: Box<EventStream<()>>,
}

// handler that does nothing
struct NoopHandler;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
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
        Self::new_with_wake_key(
            config,
            handler,
            RuntimeWakeKey::new("event-stream", 0, "standalone"),
        )
    }

    pub(crate) fn new_with_wake_key(
        config: EventStreamConfig,
        handler: Arc<dyn EventStreamHandler<State>>,
        wake_key: RuntimeWakeKey,
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

        let debounce = config.debounce.clone().map(DebouncedCommit::new);
        Self {
            config,
            between_state,
            last_handled_time: None,
            previous_event: None,
            handler,
            debounce,
            wake_key,
        }
    }

    pub(crate) fn debounce_submission(
        &mut self,
        event: &SceneGraphEvent,
        context: EventStreamContext,
        checkpoint: EventStreamAdmissionCheckpoint,
        now: Instant,
    ) -> Option<DebouncedCommitUpdate<DebouncedEvent>> {
        self.debounce.as_mut().map(|debounce| {
            debounce.submit(
                DebouncedEvent {
                    event: event.clone(),
                    context,
                    checkpoint,
                },
                now,
                &self.wake_key,
            )
        })
    }

    pub(crate) fn admission_checkpoint(&self) -> EventStreamAdmissionCheckpoint {
        EventStreamAdmissionCheckpoint {
            between_state: self.between_state.clone(),
            last_handled_time: self.last_handled_time,
            previous_event: self.previous_event.clone(),
        }
    }

    pub(crate) fn restore_admission_checkpoint(
        &mut self,
        checkpoint: EventStreamAdmissionCheckpoint,
    ) {
        self.between_state = checkpoint.between_state;
        self.last_handled_time = checkpoint.last_handled_time;
        self.previous_event = checkpoint.previous_event;
    }

    pub(crate) fn owns_runtime_wakeup(&self, key: &RuntimeWakeKey) -> bool {
        self.debounce.is_some() && self.wake_key == *key
    }

    pub(crate) fn handle_runtime_wakeup(
        &mut self,
        wake: &RuntimeWakeEvent,
        now: Instant,
    ) -> Option<DebouncedCommitUpdate<DebouncedEvent>> {
        if wake.key != self.wake_key {
            return None;
        }
        self.debounce
            .as_mut()
            .map(|debounce| debounce.handle_wakeup(wake, now))
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

        let mut end_event_context = None;

        // Handle between state
        if let Some(between) = &mut self.between_state {
            if between.start_event.is_none() {
                // Not started yet, check if this is start event
                let context = EventStreamContext::new(
                    mark_instance,
                    Some(current_snapshot.clone()),
                    None,
                    previous_event.clone(),
                );
                if between.start_stream.matches_event(event, &context, rtree) {
                    between.start_event = Some(current_snapshot);
                }
                return None;
            } else {
                // Started, check if this is end event
                start_event = between.start_event.clone();
                let context = EventStreamContext::new(
                    mark_instance,
                    Some(current_snapshot.clone()),
                    start_event.clone(),
                    previous_event.clone(),
                );
                if between.end_stream.matches_event(event, &context, rtree) {
                    between.start_event = None;
                    if self.config.emit_between_end_event {
                        end_event_context = Some(context);
                    } else {
                        return None;
                    }
                }
            }
        }

        if let Some(context) = end_event_context {
            if self.matches_event(event, &context, rtree) && self.should_handle_event(now) {
                return Some(context);
            }
            return None;
        }

        let context = EventStreamContext::new(
            mark_instance,
            Some(current_snapshot),
            start_event,
            previous_event,
        );

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

        if let Some(names) = &self.config.mark_names {
            let Some(mark_instance) = &context.mark_instance else {
                return false;
            };
            if !names
                .iter()
                .any(|target| rtree.mark_target_matches(mark_instance, target))
            {
                return false;
            }
        } else if let Some(paths) = &self.config.mark_paths {
            // Resolved paths remain the fallback for streams without stable
            // public targets. Conditional rendering may compress child indexes,
            // so a public owner-scoped target takes precedence when available.
            if let Some(mark_instance) = &context.mark_instance {
                if !paths
                    .iter()
                    .any(|path| mark_path_matches_resolved_path(&mark_instance.mark_path, path))
                {
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

fn mark_path_matches_resolved_path(scene_path: &[usize], resolved_path: &[usize]) -> bool {
    !resolved_path.is_empty()
        && (scene_path == resolved_path
            || (scene_path.len() > resolved_path.len() && scene_path.ends_with(resolved_path)))
}

#[cfg(test)]
mod tests {
    use avenger_common::cursor::CursorStyle;
    use avenger_geometry::rtree::SceneGraphRTree;
    use avenger_scenegraph::marks::mark::MarkInstance;

    use crate::{
        scene::{ModifiersState, SceneGraphEvent, SceneGraphEventType, SceneMouseDownEvent},
        window::MouseButton,
    };

    use super::*;

    fn empty_rtree() -> SceneGraphRTree {
        SceneGraphRTree::from_scene_graph(&avenger_scenegraph::scene_graph::SceneGraph {
            marks: Vec::new(),
            width: 1.0,
            height: 1.0,
            origin: [0.0, 0.0],
        })
    }

    fn mouse_down_with_path(path: Vec<usize>) -> (SceneGraphEvent, MarkInstance) {
        let mark_instance = MarkInstance {
            name: "my_box_plot.outliers".to_string(),
            mark_path: path,
            instance_index: Some(0),
        };
        (
            SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: [1.0, 2.0],
                button: MouseButton::Left,
                mark_instance: Some(mark_instance.clone()),
                modifiers: ModifiersState::default(),
            }),
            mark_instance,
        )
    }

    #[test]
    fn update_status_merge_prefers_newer_cursor() {
        let first_key = RuntimeWakeKey::new("test", 1, "first");
        let second_key = RuntimeWakeKey::new("test", 1, "second");
        let first = UpdateStatus {
            suppress_click: true,
            rerender: true,
            rebuild_geometry: false,
            cursor: Some(CursorStyle::Crosshair),
            commands: vec![RuntimeHostCommand::CancelWakeup {
                key: first_key.clone(),
            }],
            consume: false,
            admission: Some(EventAdmission::Rejected),
        };
        let second = UpdateStatus {
            suppress_click: false,
            rerender: false,
            rebuild_geometry: true,
            cursor: Some(CursorStyle::Grab),
            commands: vec![RuntimeHostCommand::CancelWakeup {
                key: second_key.clone(),
            }],
            consume: true,
            admission: Some(EventAdmission::Committed),
        };

        let merged = first.merge(&second);
        assert!(merged.rerender);
        assert!(merged.rebuild_geometry);
        assert!(merged.consume);
        assert_eq!(merged.cursor, Some(CursorStyle::Grab));
        assert_eq!(
            merged.commands,
            vec![
                RuntimeHostCommand::CancelWakeup { key: first_key },
                RuntimeHostCommand::CancelWakeup { key: second_key },
            ]
        );
    }

    #[test]
    fn resolved_mark_paths_match_scene_path_suffixes() {
        assert!(mark_path_matches_resolved_path(&[0, 1, 3], &[3]));
        assert!(mark_path_matches_resolved_path(&[3], &[3]));
        assert!(!mark_path_matches_resolved_path(&[0, 1, 4], &[3]));
        assert!(!mark_path_matches_resolved_path(&[0, 1, 3], &[]));
    }

    #[test]
    fn event_stream_resolved_mark_paths_select_only_matching_child_suffix() {
        let config = EventStreamConfig {
            types: vec![SceneGraphEventType::MouseDown],
            mark_paths: Some(vec![vec![3]]),
            ..Default::default()
        };
        let stream = EventStream::<()>::new(config, Arc::new(NoopHandler));
        let rtree = empty_rtree();

        let (matching_event, matching_mark) = mouse_down_with_path(vec![0, 1, 3]);
        let matching_context = EventStreamContext {
            mark_instance: Some(matching_mark),
            ..Default::default()
        };
        assert!(stream.matches_event(&matching_event, &matching_context, &rtree));

        let (sibling_event, sibling_mark) = mouse_down_with_path(vec![0, 1, 4]);
        let sibling_context = EventStreamContext {
            mark_instance: Some(sibling_mark),
            ..Default::default()
        };
        assert!(!stream.matches_event(&sibling_event, &sibling_context, &rtree));
    }

    #[test]
    fn public_mark_names_disambiguate_equal_scene_path_suffixes() {
        let config = EventStreamConfig {
            types: vec![SceneGraphEventType::MouseDown],
            mark_paths: Some(vec![vec![3]]),
            mark_names: Some(vec!["volume.handle".to_string()]),
            ..Default::default()
        };
        let stream = EventStream::<()>::new(config, Arc::new(NoopHandler));
        let rtree = empty_rtree();

        // Public identity stays stable when conditional widget parts compress
        // rendered child indexes away from the compiled path.
        let (event, mut matching_mark) = mouse_down_with_path(vec![0, 1, 99]);
        matching_mark.name = "volume.handle".to_string();
        let matching_context = EventStreamContext {
            mark_instance: Some(matching_mark),
            ..Default::default()
        };
        assert!(stream.matches_event(&event, &matching_context, &rtree));

        let (event, mut colliding_mark) = mouse_down_with_path(vec![9, 8, 3]);
        colliding_mark.name = "unrelated.handle".to_string();
        let colliding_context = EventStreamContext {
            mark_instance: Some(colliding_mark),
            ..Default::default()
        };
        assert!(!stream.matches_event(&event, &colliding_context, &rtree));
    }
}
