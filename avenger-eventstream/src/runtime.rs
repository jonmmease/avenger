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

/// Identity of one focused text-input session. Hosts preserve it on queued input.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputSession {
    pub owner: SmolStr,
    pub generation: u64,
}

/// Keys whose browser defaults are handled by the installed canvas controls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyboardPolicy {
    pub keys: Vec<crate::window::NamedKey>,
    pub tab_forward: bool,
    pub tab_backward: bool,
    pub text_shortcuts: bool,
}

impl KeyboardPolicy {
    /// Decide synchronously whether a DOM key's default action belongs to the canvas.
    pub fn captures(
        &self,
        key: crate::window::Key,
        modifiers: crate::scene::ModifiersState,
    ) -> bool {
        use crate::window::{Key, NamedKey};
        match key {
            Key::Named(NamedKey::Tab) => {
                if modifiers.control || modifiers.alt || modifiers.meta {
                    return false;
                }
                if modifiers.shift {
                    self.tab_backward
                } else {
                    self.tab_forward
                }
            }
            Key::Named(key) => {
                (self.text_shortcuts || !(modifiers.control || modifiers.alt || modifiers.meta))
                    && self.keys.contains(&key)
            }
            Key::Character(' ') => {
                !(modifiers.control || modifiers.alt || modifiers.meta)
                    && self.keys.contains(&NamedKey::Space)
            }
            Key::Character(ch) => {
                self.text_shortcuts
                    && (modifiers.control || modifiers.meta)
                    && matches!(ch.to_ascii_lowercase(), 'a' | 'z' | 'y')
            }
        }
    }
}

/// Host-neutral styling for one transient tooltip overlay.
///
/// Every length is expressed in root-canvas logical pixels. Colors are
/// straight-alpha linear RGBA values. The eventstream layer deliberately owns
/// only concrete presentation primitives so it does not depend on chart theme
/// or rendering crates.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeTooltipStyle {
    pub background: [f32; 4],
    pub foreground: [f32; 4],
    pub label_foreground: [f32; 4],
    pub border: [f32; 4],
    pub border_width: f32,
    pub corner_radius: f32,
    pub padding: [f32; 2],
    pub row_gap: f32,
    pub column_gap: f32,
    pub max_width: f32,
    pub font_family: SmolStr,
    pub font_size: f32,
    pub font_weight: f32,
}

impl Default for RuntimeTooltipStyle {
    fn default() -> Self {
        Self {
            background: [0.08, 0.09, 0.11, 0.96],
            foreground: [0.98, 0.98, 0.98, 1.0],
            label_foreground: [0.76, 0.78, 0.82, 1.0],
            border: [1.0, 1.0, 1.0, 0.18],
            border_width: 1.0,
            corner_radius: 4.0,
            padding: [10.0, 8.0],
            row_gap: 4.0,
            column_gap: 12.0,
            max_width: 360.0,
            font_family: SmolStr::new("sans-serif"),
            font_size: 12.0,
            font_weight: 400.0,
        }
    }
}

/// One already-formatted label/value row in a transient tooltip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTooltipRow {
    pub label: SmolStr,
    pub value: String,
}

/// Semantic tooltip content ready for a host presenter.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeTooltipPresentation {
    /// Stable tool-instance owner used to reject stale move/hide updates.
    pub owner: SmolStr,
    /// Root-canvas logical pointer position.
    pub anchor: [f32; 2],
    /// Preferred logical-pixel displacement from the pointer.
    pub offset: [f32; 2],
    pub rows: Vec<RuntimeTooltipRow>,
    pub style: RuntimeTooltipStyle,
}

/// Incremental update for the one visible tooltip associated with a canvas.
#[derive(Clone, Debug, PartialEq)]
pub enum RuntimeTooltipUpdate {
    Show(RuntimeTooltipPresentation),
    Move {
        owner: SmolStr,
        anchor: [f32; 2],
    },
    Hide {
        owner: SmolStr,
    },
    /// Host lifecycle reset that is not tied to a particular tool owner.
    Clear,
}

/// Small host-side state machine shared by native and web presenters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuntimeTooltipState {
    current: Option<RuntimeTooltipPresentation>,
}

impl RuntimeTooltipState {
    pub fn current(&self) -> Option<&RuntimeTooltipPresentation> {
        self.current.as_ref()
    }

    /// Apply an update, returning whether visible presentation state changed.
    pub fn apply(&mut self, update: RuntimeTooltipUpdate) -> bool {
        let next = match update {
            RuntimeTooltipUpdate::Show(presentation) => Some(presentation),
            RuntimeTooltipUpdate::Move { owner, anchor } => {
                let mut current = self.current.clone();
                if let Some(presentation) = current.as_mut() {
                    if presentation.owner == owner {
                        presentation.anchor = anchor;
                    }
                }
                current
            }
            RuntimeTooltipUpdate::Hide { owner } => self
                .current
                .clone()
                .filter(|presentation| presentation.owner != owner),
            RuntimeTooltipUpdate::Clear => None,
        };
        if next == self.current {
            false
        } else {
            self.current = next;
            true
        }
    }
}

#[cfg(test)]
mod tooltip_tests {
    use super::*;

    fn presentation(owner: &str, anchor: [f32; 2]) -> RuntimeTooltipPresentation {
        RuntimeTooltipPresentation {
            owner: owner.into(),
            anchor,
            offset: [12.0, 12.0],
            rows: vec![RuntimeTooltipRow {
                label: "Name".into(),
                value: "Falcon".to_string(),
            }],
            style: RuntimeTooltipStyle::default(),
        }
    }

    #[test]
    fn tooltip_state_rejects_stale_owner_updates() {
        let mut state = RuntimeTooltipState::default();
        assert!(state.apply(RuntimeTooltipUpdate::Show(presentation("new", [1.0, 2.0]))));
        assert!(!state.apply(RuntimeTooltipUpdate::Move {
            owner: "old".into(),
            anchor: [9.0, 9.0],
        }));
        assert!(!state.apply(RuntimeTooltipUpdate::Hide {
            owner: "old".into(),
        }));
        assert_eq!(state.current().unwrap().anchor, [1.0, 2.0]);

        assert!(state.apply(RuntimeTooltipUpdate::Move {
            owner: "new".into(),
            anchor: [3.0, 4.0],
        }));
        assert_eq!(state.current().unwrap().anchor, [3.0, 4.0]);
        assert!(state.apply(RuntimeTooltipUpdate::Hide {
            owner: "new".into(),
        }));
        assert!(state.current().is_none());

        assert!(state.apply(RuntimeTooltipUpdate::Show(presentation(
            "first",
            [5.0, 6.0]
        ))));
        assert!(state.apply(RuntimeTooltipUpdate::Show(presentation(
            "replacement",
            [7.0, 8.0],
        ))));
        assert_eq!(state.current().unwrap().owner, "replacement");
        assert!(state.apply(RuntimeTooltipUpdate::Clear));
        assert!(state.current().is_none());
        assert!(!state.apply(RuntimeTooltipUpdate::Clear));
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
    /// Set the owner of subsequent text, IME, and clipboard input.
    SetInputSession {
        session: Option<InputSession>,
    },
    /// Publish synchronous browser key ownership. None restores legacy host behavior.
    SetKeyboardPolicy {
        policy: Option<KeyboardPolicy>,
    },
    /// Capture the active primary mouse pointer, or release it.
    SetPointerCapture {
        captured: bool,
    },
    SetImeAllowed {
        allowed: bool,
    },
    SetImeCursorArea {
        rect: Option<LogicalRect>,
    },
    /// Cache the focused selection for synchronous browser copy/cut events.
    SetClipboardPayload {
        text: String,
    },
    WriteClipboard {
        text: String,
    },
    UpdateTooltip(RuntimeTooltipUpdate),
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
