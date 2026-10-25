use crate::manager::{EventStreamHandler, SceneTrees};
use crate::scene::{SceneGraphEvent, SceneGraphEventType};
use async_trait::async_trait;
use avenger_common::time::{Duration, Instant};
use avenger_scenegraph::marks::mark::MarkInstance;
use std::sync::Arc;

/// How a debounced stream waits for its input to settle. Matching events that come less than
/// `wait` milliseconds apart are a burst, and the handler sees the burst's last event once
/// `wait` milliseconds pass after it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebounceConfig {
    /// The milliseconds without a matching event that settle a burst.
    pub wait: u64,
    /// The most milliseconds that a burst can delay its last event, from its first.
    pub max_wait: Option<u64>,
    /// Handle a burst's first event at once. A burst of more than one event still ends with
    /// its last.
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

/// Wrapper around a filter function that supports Debug formatting
#[derive(Clone)]
pub struct EventStreamFilter(pub Arc<dyn Fn(&SceneGraphEvent) -> bool + Send + Sync + 'static>);

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

    /// If specified, only events matching all of the filters will be included
    pub filter: Option<Vec<EventStreamFilter>>,

    /// If specified, only events that occur after the start stream has been triggered
    /// and before the end stream has been triggered will be included. A canceled press also
    /// ends a window whose start stream hears presses.
    pub between: Option<(Box<EventStreamConfig>, Box<EventStreamConfig>)>,

    /// If specified, only events associated with the specified mark paths will be included
    pub mark_paths: Option<Vec<Vec<usize>>>,

    /// Minimum time (in milliseconds) between events
    pub throttle: Option<u64>,

    /// If specified, the handler sees each burst of matching events once it settles, as
    /// [`DebounceConfig`] describes. The handler's [`Disposition`] applies to each event as it
    /// comes, so a consumed event stops at the stream though the handler sees it later.
    pub debounce: Option<DebounceConfig>,
}

/// What a stream does with an event that its config matches, as its handler decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// Ignore the event, as if the config didn't match it.
    Ignore,
    /// Handle the event, and let it go on to later streams in the scene and to lower scenes.
    Handle,
    /// Handle the event, and keep it from later streams in the scene and from lower scenes.
    /// The scene that owns a press still receives its pointer events.
    Consume,
}

/// The parts of a scene to rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Rebuild the graph and leave the hit-testing tree stale, as during a pan.
    Scene,
    /// Rebuild both the graph and its hit-testing tree.
    Tree,
}

/// Named scene rebuild requests, each with a rebuild level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rebuild {
    scenes: Vec<(String, Level)>,
}

impl Rebuild {
    /// Create an empty set of rebuild requests.
    pub fn none() -> Self {
        Self::default()
    }

    /// Request a graph rebuild for `name`, leaving its tree stale.
    pub fn scene(name: &str) -> Self {
        Self::none().and_scene(name)
    }

    /// Request a graph and tree rebuild for `name`.
    pub fn tree(name: &str) -> Self {
        Self::none().and_tree(name)
    }

    /// Add a graph rebuild for `name`, preserving any existing tree rebuild request.
    pub fn and_scene(self, name: &str) -> Self {
        self.and(name, Level::Scene)
    }

    /// Add a graph and tree rebuild for `name`.
    pub fn and_tree(self, name: &str) -> Self {
        self.and(name, Level::Tree)
    }

    fn and(mut self, name: &str, level: Level) -> Self {
        self.merge(Rebuild {
            scenes: vec![(name.to_string(), level)],
        });
        self
    }

    /// Merge `other`, keeping the higher rebuild level for each scene.
    pub fn merge(&mut self, other: Rebuild) {
        for (name, level) in other.scenes {
            match self.scenes.iter_mut().find(|(own, _)| *own == name) {
                Some((_, own)) => *own = (*own).max(level),
                None => self.scenes.push((name, level)),
            }
        }
    }

    /// Return whether no scenes are requested.
    pub fn is_empty(&self) -> bool {
        self.scenes.is_empty()
    }

    /// Iterate over scene names and rebuild levels in first-request order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Level)> + '_ {
        (self.scenes.iter()).map(|(name, level)| (name.as_str(), *level))
    }
}

/// Internal struct representing the state of an event stream and it's handler
#[derive(Clone)]
pub(crate) struct EventStream<State: Send + Sync + 'static> {
    /// The scene receiving these events, indexed from the bottom.
    pub(crate) scene: usize,
    pub(crate) config: EventStreamConfig,
    pub(crate) between_state: Option<BetweenState>,
    pub(crate) last_handled_time: Option<Instant>,
    pub(crate) handler: Arc<dyn EventStreamHandler<State>>,
    /// The burst that a debounced stream takes in.
    burst: Burst,
}

/// The burst of events that a debounced stream takes in, until it settles.
#[derive(Clone, Default)]
pub(crate) struct Burst {
    /// When the burst's first event came, while a burst lasts.
    started: Option<Instant>,
    /// When the burst settles unless another event comes, while it lasts.
    settles_at: Option<Instant>,
    /// The burst's last event, when the handler hasn't seen it.
    last: Option<SceneGraphEvent>,
}

#[derive(Clone)]
pub(crate) struct BetweenState {
    started: bool,
    start_stream: Box<EventStream<()>>,
    end_stream: Box<EventStream<()>>,
}

// handler that does nothing
struct NoopHandler;

#[async_trait]
impl EventStreamHandler<()> for NoopHandler {
    async fn handle(&self, _: &SceneGraphEvent, _: &mut (), _: &SceneTrees<'_>) -> Rebuild {
        Rebuild::none()
    }
}

impl<State: Send + Sync + 'static> EventStream<State> {
    pub(crate) fn new(
        scene: usize,
        config: EventStreamConfig,
        handler: Arc<dyn EventStreamHandler<State>>,
    ) -> Self {
        // Initialize between_state if config.between is specified
        let between_state = config
            .between
            .as_ref()
            .map(|(start_cfg, end_cfg)| BetweenState {
                started: false,
                start_stream: Box::new(EventStream::new(
                    scene,
                    start_cfg.as_ref().clone(),
                    Arc::new(NoopHandler),
                )),
                end_stream: Box::new(EventStream::new(
                    scene,
                    end_cfg.as_ref().clone(),
                    Arc::new(NoopHandler),
                )),
            });

        Self {
            scene,
            config,
            between_state,
            last_handled_time: None,
            handler,
            burst: Burst::default(),
        }
    }

    /// Take a matching event into a debounced stream's burst, at `now`. Returns whether the
    /// handler handles the event at once, as a burst's first event with `leading`. A stream that
    /// isn't debounced returns none.
    pub(crate) fn debounce(&mut self, event: &SceneGraphEvent, now: Instant) -> Option<bool> {
        let config = self.config.debounce.as_ref()?;
        let first = self.burst.started.is_none();
        let started = *self.burst.started.get_or_insert(now);
        let at_once = first && config.leading;
        self.burst.last = (!at_once).then(|| event.clone());
        let settles = now + Duration::from_millis(config.wait);
        self.burst.settles_at = Some(match config.max_wait {
            Some(max_wait) => settles.min(started + Duration::from_millis(max_wait)),
            None => settles,
        });
        Some(at_once)
    }

    /// When the stream's burst settles unless another event comes, while a burst lasts.
    pub(crate) fn settles_at(&self) -> Option<Instant> {
        self.burst.settles_at
    }

    /// End a debounced stream's burst, which settled. Returns its last event when the handler
    /// hasn't seen it.
    pub(crate) fn settle(&mut self) -> Option<SceneGraphEvent> {
        self.burst.started = None;
        self.burst.settles_at = None;
        self.burst.last.take()
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
                // Started, check if this is end event. A canceled press also ends a window that a
                // press started, since the press's release never comes.
                let cancelled = matches!(event, SceneGraphEvent::MouseCancel(_))
                    && (between.start_stream.config.types)
                        .contains(&SceneGraphEventType::MouseDown);
                if cancelled || between.end_stream.matches_event(event, mark_instance) {
                    between.started = false;
                    return false;
                }
            }
        }

        // Check if event matches and throttling allows it
        self.matches_event(event, mark_instance) && self.should_handle_event(now)
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
                if !filter.0(event) {
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
