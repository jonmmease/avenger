use crate::manager::EventStreamHandler;
use crate::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_common::time::{Duration, Instant};
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

/// Which events a stream hears, and how often. The stream's handler decides what to do with
/// each, through [`EventStreamHandler::disposition`].
#[derive(Clone, Default, Debug)]
pub struct EventStreamConfig {
    /// The event types that the stream hears.
    pub types: Vec<SceneGraphEventType>,

    /// The fewest milliseconds between the events that the stream handles, if any.
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

impl<State: Send + Sync + 'static> EventStream<State> {
    pub(crate) fn new(
        scene: usize,
        config: EventStreamConfig,
        handler: Arc<dyn EventStreamHandler<State>>,
    ) -> Self {
        Self {
            scene,
            config,
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

    /// Whether the stream hears `event` at `now`: its configuration lists the event's type, and
    /// its throttle allows it.
    pub(crate) fn matches(&self, event: &SceneGraphEvent, now: Instant) -> bool {
        let throttled =
            (self.config.throttle.zip(self.last_handled_time)).is_some_and(|(throttle, last)| {
                now.duration_since(last) < Duration::from_millis(throttle)
            });
        self.config.types.contains(&event.event_type()) && !throttled
    }
}
