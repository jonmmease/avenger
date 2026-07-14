use avenger_common::time::{Duration, Instant};
use smol_str::SmolStr;

/// Stable identity for a host wake-up request.
///
/// Native-widget users populate this with their instance namespace,
/// attachment epoch, and a purpose local to that attachment. Event streams
/// use the same shape for their manager-local debounce timers.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeWakeKey {
    pub namespace: SmolStr,
    pub attachment_epoch: u64,
    pub purpose: SmolStr,
}

impl RuntimeWakeKey {
    pub fn new(
        namespace: impl Into<SmolStr>,
        attachment_epoch: u64,
        purpose: impl Into<SmolStr>,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            attachment_epoch,
            purpose: purpose.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeWakeEvent {
    pub key: RuntimeWakeKey,
    pub generation: u64,
}

/// A finite rectangle in root-canvas logical pixels with a top-left origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl LogicalRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Option<Self> {
        let values = [x, y, width, height];
        (values.iter().all(|value| value.is_finite()) && width >= 0.0 && height >= 0.0).then_some(
            Self {
                x,
                y,
                width,
                height,
            },
        )
    }

    pub fn x(self) -> f32 {
        self.x
    }

    pub fn y(self) -> f32 {
        self.y
    }

    pub fn width(self) -> f32 {
        self.width
    }

    pub fn height(self) -> f32 {
        self.height
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RuntimeHostCommand {
    RequestWakeup {
        key: RuntimeWakeKey,
        deadline: Instant,
        generation: u64,
    },
    CancelWakeup {
        key: RuntimeWakeKey,
    },
    SetImeAllowed {
        allowed: bool,
    },
    SetImeCursorArea {
        rect: Option<LogicalRect>,
    },
    WriteClipboard {
        text: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebounceConfig {
    /// The number of milliseconds of silence before the trailing value commits.
    pub wait: u64,
    /// The maximum time a burst may defer its trailing commit.
    pub max_wait: Option<u64>,
    /// Commit the first value in a burst immediately.
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
pub struct DebouncedCommitUpdate<T> {
    pub commit: Option<T>,
    pub commands: Vec<RuntimeHostCommand>,
}

impl<T> Default for DebouncedCommitUpdate<T> {
    fn default() -> Self {
        Self {
            commit: None,
            commands: Vec::new(),
        }
    }
}

/// Host-neutral trailing/leading debounce state.
///
/// The helper never creates a timer. It returns keyed host commands, and the
/// host later feeds the matching [`RuntimeWakeEvent`] back through dispatch.
#[derive(Clone, Debug)]
pub struct DebouncedCommit<T> {
    config: DebounceConfig,
    pending: Option<T>,
    burst_started_at: Option<Instant>,
    deadline: Option<Instant>,
    wake_key: Option<RuntimeWakeKey>,
    generation: u64,
}

impl<T: Clone> DebouncedCommit<T> {
    pub fn new(config: DebounceConfig) -> Self {
        Self {
            config,
            pending: None,
            burst_started_at: None,
            deadline: None,
            wake_key: None,
            generation: 0,
        }
    }

    pub fn submit(
        &mut self,
        value: T,
        now: Instant,
        key: &RuntimeWakeKey,
    ) -> DebouncedCommitUpdate<T> {
        if self.config.wait == 0 {
            let mut update = self.cancel(key);
            update.commit = Some(value);
            return update;
        }

        let starts_burst = self.burst_started_at.is_none();
        let burst_started_at = *self.burst_started_at.get_or_insert(now);
        let commit = (starts_burst && self.config.leading).then(|| value.clone());
        self.pending = if commit.is_some() { None } else { Some(value) };
        self.generation = self.generation.wrapping_add(1).max(1);

        let silence_deadline = now + Duration::from_millis(self.config.wait);
        let deadline = self
            .config
            .max_wait
            .map(|max_wait| burst_started_at + Duration::from_millis(max_wait))
            .map_or(silence_deadline, |max_deadline| {
                silence_deadline.min(max_deadline)
            });
        self.deadline = Some(deadline);
        let previous_key = self.wake_key.replace(key.clone());

        let mut commands = Vec::with_capacity(2);
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous != key)
        {
            commands.push(RuntimeHostCommand::CancelWakeup {
                key: previous_key.expect("checked above"),
            });
        }
        commands.push(RuntimeHostCommand::RequestWakeup {
            key: key.clone(),
            deadline,
            generation: self.generation,
        });

        DebouncedCommitUpdate { commit, commands }
    }

    pub fn handle_wakeup(
        &mut self,
        wake: &RuntimeWakeEvent,
        now: Instant,
    ) -> DebouncedCommitUpdate<T> {
        if self.wake_key.as_ref() != Some(&wake.key)
            || wake.generation != self.generation
            || self.deadline.is_none()
        {
            return DebouncedCommitUpdate::default();
        }
        let deadline = self.deadline.expect("checked above");
        if now < deadline {
            return DebouncedCommitUpdate {
                commit: None,
                commands: vec![RuntimeHostCommand::RequestWakeup {
                    key: wake.key.clone(),
                    deadline,
                    generation: self.generation,
                }],
            };
        }

        let commit = self.pending.take();
        self.reset();
        DebouncedCommitUpdate {
            commit,
            commands: Vec::new(),
        }
    }

    pub fn flush(&mut self, key: &RuntimeWakeKey) -> DebouncedCommitUpdate<T> {
        let commit = self.pending.take();
        let had_deadline = self.deadline.is_some();
        let scheduled_key = self.wake_key.clone().unwrap_or_else(|| key.clone());
        self.reset();
        DebouncedCommitUpdate {
            commit,
            commands: had_deadline
                .then_some(RuntimeHostCommand::CancelWakeup { key: scheduled_key })
                .into_iter()
                .collect(),
        }
    }

    pub fn cancel(&mut self, key: &RuntimeWakeKey) -> DebouncedCommitUpdate<T> {
        let had_deadline = self.deadline.is_some();
        let scheduled_key = self.wake_key.clone().unwrap_or_else(|| key.clone());
        self.reset();
        DebouncedCommitUpdate {
            commit: None,
            commands: had_deadline
                .then_some(RuntimeHostCommand::CancelWakeup { key: scheduled_key })
                .into_iter()
                .collect(),
        }
    }

    pub fn pending_generation(&self) -> Option<u64> {
        self.deadline.map(|_| self.generation)
    }

    pub fn pending_deadline(&self) -> Option<Instant> {
        self.deadline
    }

    fn reset(&mut self) {
        self.pending = None;
        self.burst_started_at = None;
        self.deadline = None;
        self.wake_key = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> RuntimeWakeKey {
        RuntimeWakeKey::new("test", 7, "commit")
    }

    fn requested(update: &DebouncedCommitUpdate<&'static str>) -> (Instant, u64) {
        let RuntimeHostCommand::RequestWakeup {
            deadline,
            generation,
            ..
        } = &update.commands[0]
        else {
            panic!("expected wakeup request")
        };
        (*deadline, *generation)
    }

    #[test]
    fn trailing_debounce_replaces_and_ignores_stale_wakes() {
        let start = Instant::now();
        let key = key();
        let mut debounce = DebouncedCommit::new(DebounceConfig::new(20));

        let first = debounce.submit("first", start, &key);
        let (first_deadline, first_generation) = requested(&first);
        assert!(first.commit.is_none());

        let second = debounce.submit("second", start + Duration::from_millis(10), &key);
        let (second_deadline, second_generation) = requested(&second);
        assert!(second_deadline > first_deadline);
        assert!(second_generation > first_generation);

        let stale = debounce.handle_wakeup(
            &RuntimeWakeEvent {
                key: key.clone(),
                generation: first_generation,
            },
            second_deadline,
        );
        assert!(stale.commit.is_none());

        let ready = debounce.handle_wakeup(
            &RuntimeWakeEvent {
                key,
                generation: second_generation,
            },
            second_deadline,
        );
        assert_eq!(ready.commit, Some("second"));
        assert_eq!(debounce.pending_generation(), None);
    }

    #[test]
    fn leading_and_max_wait_have_deterministic_deadlines() {
        let start = Instant::now();
        let key = key();
        let mut debounce = DebouncedCommit::new(DebounceConfig {
            wait: 20,
            max_wait: Some(25),
            leading: true,
        });

        let first = debounce.submit("first", start, &key);
        assert_eq!(first.commit, Some("first"));
        let second = debounce.submit("second", start + Duration::from_millis(15), &key);
        let (deadline, generation) = requested(&second);
        assert_eq!(deadline.duration_since(start), Duration::from_millis(25));

        let ready = debounce.handle_wakeup(&RuntimeWakeEvent { key, generation }, deadline);
        assert_eq!(ready.commit, Some("second"));
    }

    #[test]
    fn flush_and_cancel_invalidate_the_pending_generation() {
        let start = Instant::now();
        let key = key();
        let mut debounce = DebouncedCommit::new(DebounceConfig::new(20));

        debounce.submit("flush", start, &key);
        let flushed = debounce.flush(&key);
        assert_eq!(flushed.commit, Some("flush"));
        assert!(matches!(
            flushed.commands.as_slice(),
            [RuntimeHostCommand::CancelWakeup { .. }]
        ));

        debounce.submit("cancel", start, &key);
        let cancelled = debounce.cancel(&key);
        assert!(cancelled.commit.is_none());
        assert_eq!(debounce.pending_generation(), None);
    }
}
