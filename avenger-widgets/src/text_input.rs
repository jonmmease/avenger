use crate::{Options, Rect, TextStyle, WidgetId, WidgetSpec};
use avenger_common::time::{Duration, Instant};
use avenger_eventstream::runtime::{DebounceConfig, DebouncedCommit, InputSession};
use avenger_text::text_edit::{
    SelectionState, ShapedLine, SingleLineEditor, normalize_single_line,
};

/// When draft changes request a downstream commit. Draft change events stay immediate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TextCommitPolicy {
    #[default]
    Immediate,
    Debounced(Duration),
    OnEnterOrBlur,
}
/// A plain single-line source field. Typst validation belongs to the application.
#[derive(Clone, Debug)]
pub struct TextInput {
    pub(crate) options: Options,
    pub(crate) value: String,
    pub(crate) placeholder: String,
    pub(crate) read_only: bool,
    pub(crate) invalid: bool,
    pub(crate) policy: TextCommitPolicy,
    pub(crate) text_style: Option<TextStyle>,
}
impl TextInput {
    pub fn new(id: impl Into<WidgetId>, value: impl Into<String>) -> Self {
        Self {
            options: Options::new(id),
            value: normalize_single_line(&value.into()),
            placeholder: String::new(),
            read_only: false,
            invalid: false,
            policy: TextCommitPolicy::Immediate,
            text_style: None,
        }
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.options.enabled = enabled;
        self
    }
    pub fn semantic_name(mut self, name: impl Into<String>) -> Self {
        self.options.semantic_name = Some(name.into());
        self
    }
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = normalize_single_line(&text.into());
        self
    }
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }
    pub fn commit_policy(mut self, policy: TextCommitPolicy) -> Self {
        self.policy = policy;
        self
    }
    /// Override the field's uniform plain-text typography.
    pub fn text_style(mut self, style: TextStyle) -> Self {
        self.text_style = Some(style);
        self
    }
}
impl From<TextInput> for WidgetSpec {
    fn from(value: TextInput) -> Self {
        Self::TextInput(value)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCommitReason {
    Immediate,
    Debounced,
    Enter,
    Blur,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCancelReason {
    Escape,
    Composition,
    Disabled,
    Removed,
    Reset,
}
/// Shortcut conventions selected by an adapter. WASM hosts can inspect navigator.platform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextShortcuts {
    #[default]
    Control,
    Mac,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Snapshot {
    pub text: String,
    pub selection: SelectionState,
}
impl Snapshot {
    pub fn new(editor: &SingleLineEditor) -> Self {
        Self {
            text: editor.text().into(),
            selection: editor.selection(),
        }
    }
    pub fn restore(&self, editor: &mut SingleLineEditor) {
        *editor = SingleLineEditor::new(&self.text);
        editor.set_selection(self.selection);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditClass {
    Typing,
    Backward,
    Forward,
    Separate,
}
#[derive(Clone, Debug)]
struct HistoryEntry {
    before: Snapshot,
    after: Snapshot,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct History {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    last: Option<(EditClass, Instant)>,
}
impl History {
    pub fn boundary(&mut self) {
        self.last = None;
    }
    pub fn push(&mut self, before: Snapshot, after: Snapshot, class: EditClass, now: Instant) {
        if before.text == after.text {
            return;
        }
        let coalesce = self.last.is_some_and(|(c, time)| {
            c == class
                && class != EditClass::Separate
                && now.saturating_duration_since(time) < Duration::from_secs(1)
        }) && self.undo.last().is_some_and(|entry| entry.after == before);
        if coalesce {
            self.undo.last_mut().unwrap().after = after;
        } else {
            if self.undo.len() == 100 {
                self.undo.remove(0);
            }
            self.undo.push(HistoryEntry { before, after });
        }
        self.redo.clear();
        self.last = Some((class, now));
    }
    pub fn restore(&mut self, redo: bool) -> Option<Snapshot> {
        self.boundary();
        if redo {
            let e = self.redo.pop()?;
            let s = e.after.clone();
            self.undo.push(e);
            Some(s)
        } else {
            let e = self.undo.pop()?;
            let s = e.before.clone();
            self.redo.push(e);
            Some(s)
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct TextLayout {
    pub inner: Rect,
    pub clip: Rect,
    pub origin: [f32; 2],
    pub line: ShapedLine,
}
#[derive(Clone, Debug)]
pub(crate) struct TextState {
    pub editor: SingleLineEditor,
    pub baseline: String,
    pub pending: bool,
    pub debounce: DebouncedCommit<String>,
    pub composition: Option<Snapshot>,
    pub deferred: Option<String>,
    pub history: History,
    pub session: Option<InputSession>,
    pub caret_visible: bool,
    pub blink_generation: u64,
    pub scroll: f32,
    pub layout: Option<TextLayout>,
    pub last_click: Option<(Instant, [f32; 2], u8)>,
    pub drag_x: Option<f32>,
    pub scroll_generation: u64,
}
impl TextState {
    pub fn new(spec: &TextInput) -> Self {
        Self {
            editor: SingleLineEditor::new(&spec.value),
            baseline: spec.value.clone(),
            pending: false,
            debounce: DebouncedCommit::new(debounce_config(&spec.policy)),
            composition: None,
            deferred: None,
            history: History::default(),
            session: None,
            caret_visible: true,
            blink_generation: 0,
            scroll: 0.0,
            layout: None,
            last_click: None,
            drag_x: None,
            scroll_generation: 0,
        }
    }
}
pub(crate) fn debounce_config(policy: &TextCommitPolicy) -> DebounceConfig {
    DebounceConfig::new(match policy {
        TextCommitPolicy::Debounced(d) => (d.as_millis()
            + u128::from(d.subsec_nanos() % 1_000_000 != 0))
        .min(u128::from(u64::MAX)) as u64,
        _ => 0,
    })
}
