//! Native single-line text input.

use std::{
    collections::VecDeque,
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
};

use avenger_chart::{
    plot::ScopedParamAssignment,
    prelude::{
        AvengerChartError, CompiledNativeWidgetSpec, CompiledParamSpec, NativeWidget,
        NativeWidgetCtx, NativeWidgetEnvironment, NativeWidgetEvaluationIntent, NativeWidgetEvent,
        NativeWidgetFactory, NativeWidgetFactoryContext, NativeWidgetInstance,
        NativeWidgetMeasureSpec, NativeWidgetMeasurement, NativeWidgetRegistry, NativeWidgetScene,
        NativeWidgetStateSnapshot, NativeWidgetStateSpec, Param, ResolvedWidgetAxisSize,
        WidgetStyleProperty,
    },
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    time::{Duration, Instant},
    types::StrokeCap,
    value::ScalarOrArray,
};
use avenger_eventstream::{
    runtime::LogicalRect,
    scene::{SceneGraphEvent, SceneKeyPressEvent},
    window::{ClipboardEvent, ImeEvent, Key, MouseButton, NamedKey},
};
use avenger_scenegraph::marks::{
    mark::SceneMark, rect::SceneRectMark, rule::SceneRuleMark, text::SceneTextMark,
};
use avenger_text::{
    TextEngine, empty_label_params,
    measurement::TextMeasurementConfig,
    text_edit::{
        Action, Cursor, Motion, SelectionState, SingleLineEditor, cursor_rect_for_offset,
        selection_rects,
    },
    types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline, TextSyntaxMode},
};
use datafusion::{common::ScalarValue, logical_expr::Expr};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::style::BuiltinWidgetKind;

const SCHEMA_VERSION: u32 = 1;
const DEFAULT_DEBOUNCE_MS: u64 = 150;
const COMMIT_WAKE: &str = "text-input-commit";
const SCROLL_PAD: f32 = 4.0;
const CLICK_GAP: Duration = Duration::from_millis(300);
const CLICK_DISTANCE: f32 = 6.0;

/// When the editing buffer is copied into the document parameter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextCommit {
    /// Commit after a short quiet period, and flush on Enter or blur.
    #[default]
    OnChange,
    /// Keep edits local until Enter or blur.
    OnEnterOrBlur,
}

/// Authoring description for Avenger's built-in native text input.
pub struct TextInput {
    id: String,
    placeholder: String,
    commit: TextCommit,
    debounce_ms: u64,
    value: Param,
    cursor_live: AtomicBool,
    selected_text_live: AtomicBool,
}

impl TextInput {
    /// Create an empty text input with a generated `<id>__value` parameter.
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            value: Param::new(format!("{id}__value"), ""),
            id,
            placeholder: String::new(),
            commit: TextCommit::OnChange,
            debounce_ms: DEFAULT_DEBOUNCE_MS,
            cursor_live: AtomicBool::new(false),
            selected_text_live: AtomicBool::new(false),
        }
    }

    /// Set the hint shown while the committed value is empty.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Choose when edits are published to the value parameter.
    pub fn commit(mut self, commit: TextCommit) -> Self {
        self.commit = commit;
        self
    }

    /// Set the quiet period used by [`TextCommit::OnChange`], in milliseconds.
    pub fn debounce(mut self, milliseconds: u64) -> Self {
        self.debounce_ms = milliseconds;
        self
    }

    /// Set the generated value parameter's initial string.
    pub fn initial_value(mut self, value: impl Into<String>) -> Self {
        self.value.default = ScalarValue::Utf8(Some(value.into()));
        self
    }

    /// Replace the generated value parameter with an author-supplied parameter.
    pub fn value_param(mut self, param: Param) -> Self {
        self.value = param;
        self
    }

    /// Return the parameter that stores the committed value.
    pub fn param(&self) -> &Param {
        &self.value
    }

    /// Return an expression that reads the committed value.
    pub fn value(&self) -> Expr {
        self.value.expr()
    }

    /// Opt into publishing the committed-text cursor as a grapheme index.
    pub fn cursor_position(&self) -> Expr {
        self.cursor_live.store(true, Ordering::Relaxed);
        Param::new(self.cursor_param_name(), 0_u64).expr()
    }

    /// Opt into publishing the selected committed text.
    pub fn selected_text(&self) -> Expr {
        self.selected_text_live.store(true, Ordering::Relaxed);
        Param::new(self.selected_text_param_name(), "").expr()
    }

    fn cursor_param_name(&self) -> String {
        format!("{}__cursor", self.id)
    }

    fn selected_text_param_name(&self) -> String {
        format!("{}__selected_text", self.id)
    }
}

impl NativeWidget for TextInput {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        "text-input"
    }

    fn schema_version(&self) -> u32 {
        SCHEMA_VERSION
    }

    fn payload(&self) -> serde_json::Value {
        serde_json::to_value(TextInputPayload {
            placeholder: self.placeholder.clone(),
            commit: self.commit,
            debounce_ms: self.debounce_ms,
            value_param: self.value.name.clone(),
            cursor_param: self
                .cursor_live
                .load(Ordering::Relaxed)
                .then(|| self.cursor_param_name()),
            selected_text_param: self
                .selected_text_live
                .load(Ordering::Relaxed)
                .then(|| self.selected_text_param_name()),
        })
        .expect("TextInput payload is serializable")
    }

    fn measure(&self) -> NativeWidgetMeasureSpec {
        NativeWidgetMeasureSpec::Registry
    }

    fn state(&self) -> NativeWidgetStateSpec {
        let mut params = vec![CompiledParamSpec::shared(&self.value)];
        if self.cursor_live.load(Ordering::Relaxed) {
            params.push(CompiledParamSpec::shared(&Param::new(
                self.cursor_param_name(),
                0_u64,
            )));
        }
        if self.selected_text_live.load(Ordering::Relaxed) {
            params.push(CompiledParamSpec::shared(&Param::new(
                self.selected_text_param_name(),
                "",
            )));
        }
        NativeWidgetStateSpec::try_new(params).expect("TextInput state names are distinct")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TextInputPayload {
    placeholder: String,
    commit: TextCommit,
    debounce_ms: u64,
    value_param: String,
    cursor_param: Option<String>,
    selected_text_param: Option<String>,
}

/// Runtime factory for the built-in `text-input` native kind.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextInputFactory;

impl NativeWidgetFactory for TextInputFactory {
    fn kind(&self) -> &'static str {
        "text-input"
    }

    fn supported_schema_versions(&self) -> std::ops::RangeInclusive<u32> {
        SCHEMA_VERSION..=SCHEMA_VERSION
    }

    fn part_manifests(
        &self,
        _spec: &CompiledNativeWidgetSpec,
        _payload: &serde_json::Value,
    ) -> Result<Vec<avenger_chart::prelude::WidgetPartManifest>, AvengerChartError> {
        Ok(BuiltinWidgetKind::TextInput.part_manifest())
    }

    fn measure(
        &self,
        spec: &CompiledNativeWidgetSpec,
        _payload: &serde_json::Value,
        ctx: &NativeWidgetFactoryContext<'_>,
    ) -> Result<NativeWidgetMeasurement, AvengerChartError> {
        let eval =
            avenger_chart_core::theme::eval::EvalContext::new(ctx.params, ctx.base_font_size);
        let length = |property: WidgetStyleProperty| {
            ctx.styles
                .host
                .values
                .get(&property)
                .ok_or_else(|| AvengerChartError::InvalidWidgetStyle {
                    widget_id: spec.id.clone(),
                    property: property.name().to_string(),
                    message: "required by native TextInput measurement".to_string(),
                })?
                .eval_as_length(&eval)
                .map(|value| value as f32)
                .map_err(|error| AvengerChartError::InvalidWidgetStyle {
                    widget_id: spec.id.clone(),
                    property: property.name().to_string(),
                    message: error.to_string(),
                })
        };
        let height = length(WidgetStyleProperty::Height)?;
        let min_width = length(WidgetStyleProperty::MinWidth)?;
        Ok(NativeWidgetMeasurement {
            width: ResolvedWidgetAxisSize {
                min_px: min_width,
                preferred_px: min_width,
                stretch: 1.0,
            },
            height: ResolvedWidgetAxisSize {
                min_px: height,
                preferred_px: height,
                stretch: 0.0,
            },
        })
    }

    fn create(
        &self,
        spec: &CompiledNativeWidgetSpec,
        payload: &serde_json::Value,
    ) -> Result<Box<dyn NativeWidgetInstance>, AvengerChartError> {
        let payload: TextInputPayload =
            serde_json::from_value(payload.clone()).map_err(|error| {
                AvengerChartError::InvalidArgument(format!(
                    "TextInput '{}' has invalid payload: {error}",
                    spec.id
                ))
            })?;
        let engine = TextEngine::with_default_config().map_err(|error| {
            AvengerChartError::InternalError(format!(
                "failed to initialize TextInput fonts: {error}"
            ))
        })?;
        Ok(Box::new(TextInputInstance::new(payload, engine)))
    }
}

/// Register Avenger's built-in native widget kinds in an existing registry.
pub fn register_native_widgets(
    registry: &mut NativeWidgetRegistry,
) -> Result<(), AvengerChartError> {
    registry.register(TextInputFactory)
}

#[derive(Clone, Debug, PartialEq)]
struct TextInputStyle {
    frame_size: [f32; 2],
    inset: f32,
    box_fill: [f32; 4],
    box_stroke: [f32; 4],
    border_width: f32,
    radius: f32,
    text_color: [f32; 4],
    placeholder_color: [f32; 4],
    selection_color: [f32; 4],
    selection_opacity: f32,
    caret_color: [f32; 4],
    caret_width: f32,
    preedit_color: [f32; 4],
    preedit_width: f32,
    focus_color: [f32; 4],
    focus_width: f32,
    focus_gap: f32,
    font: String,
    font_size: f32,
    font_weight: FontWeight,
}

#[derive(Clone, Debug, PartialEq)]
struct EditState {
    selection: SelectionState,
    text: String,
}

#[derive(Debug, Default)]
struct Undoer {
    undos: Vec<EditState>,
    redos: Vec<EditState>,
    last_observed: Option<EditState>,
    saved: Option<EditState>,
    changed_at: Option<Instant>,
    saved_at: Option<Instant>,
}

impl Undoer {
    fn feed(&mut self, now: Instant, state: EditState) {
        let Some(observed) = self.last_observed.as_ref() else {
            self.last_observed = Some(state.clone());
            self.saved = Some(state);
            self.changed_at = Some(now);
            self.saved_at = Some(now);
            return;
        };
        if observed != &state {
            self.last_observed = Some(state.clone());
            self.changed_at = Some(now);
            self.redos.clear();
        }
        let stable = self.changed_at.is_some_and(|changed| {
            now.saturating_duration_since(changed) >= Duration::from_secs(1)
        });
        let auto = self
            .saved_at
            .is_some_and(|saved| now.saturating_duration_since(saved) >= Duration::from_secs(30));
        if (stable || auto) && self.saved.as_ref() != Some(&state) {
            if let Some(saved) = self.saved.replace(state) {
                self.undos.push(saved);
                if self.undos.len() > 100 {
                    self.undos.remove(0);
                }
            }
            self.redos.clear();
            self.saved_at = Some(now);
        }
    }

    fn undo(&mut self, current: EditState) -> Option<EditState> {
        let target = if self.saved.as_ref().is_some_and(|saved| saved != &current) {
            self.saved.clone()
        } else {
            self.undos.pop()
        }?;
        self.redos.push(current);
        self.saved = Some(target.clone());
        self.last_observed = Some(target.clone());
        Some(target)
    }

    fn redo(&mut self, current: EditState) -> Option<EditState> {
        let target = self.redos.pop()?;
        self.undos.push(current);
        self.saved = Some(target.clone());
        self.last_observed = Some(target.clone());
        Some(target)
    }

    fn clear(&mut self, now: Instant, state: EditState) {
        *self = Self::default();
        self.feed(now, state);
    }
}

#[derive(Clone, Copy, Debug)]
struct ClickState {
    at: Instant,
    point: [f32; 2],
    count: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StandardShortcut {
    SelectAll,
    Undo,
    Redo,
}

fn text_shortcut(key: &SceneKeyPressEvent, is_apple: bool) -> Option<Action> {
    let command = if is_apple {
        key.modifiers.meta
    } else {
        key.modifiers.control
    };
    let word = if is_apple {
        key.modifiers.alt
    } else {
        key.modifiers.control
    };
    let extend = key.modifiers.shift;
    match key.key {
        Key::Named(NamedKey::ArrowLeft) if is_apple && command => Some(Action::Motion {
            motion: Motion::Start,
            extend,
        }),
        Key::Named(NamedKey::ArrowRight) if is_apple && command => Some(Action::Motion {
            motion: Motion::End,
            extend,
        }),
        Key::Named(NamedKey::ArrowLeft) => Some(Action::Motion {
            motion: if word { Motion::WordLeft } else { Motion::Left },
            extend,
        }),
        Key::Named(NamedKey::ArrowRight) => Some(Action::Motion {
            motion: if word {
                Motion::WordRight
            } else {
                Motion::Right
            },
            extend,
        }),
        Key::Named(NamedKey::Home) => Some(Action::Motion {
            motion: Motion::Start,
            extend,
        }),
        Key::Named(NamedKey::End) => Some(Action::Motion {
            motion: Motion::End,
            extend,
        }),
        Key::Named(NamedKey::Backspace) if is_apple && command => Some(Action::DeleteToStart),
        Key::Named(NamedKey::Backspace) if word => Some(Action::DeleteWordBack),
        Key::Named(NamedKey::Delete) if word => Some(Action::DeleteWordForward),
        Key::Named(NamedKey::Backspace) => Some(Action::Backspace),
        Key::Named(NamedKey::Delete) => Some(Action::Delete),
        Key::Named(NamedKey::Escape) => Some(Action::Escape),
        _ => None,
    }
}

fn standard_shortcut(key: &SceneKeyPressEvent, is_apple: bool) -> Option<StandardShortcut> {
    let command = if is_apple {
        key.modifiers.meta
    } else {
        key.modifiers.control
    };
    match key.key {
        Key::Character(ch) if command && ch.eq_ignore_ascii_case(&'a') => {
            Some(StandardShortcut::SelectAll)
        }
        Key::Character(ch) if command && ch.eq_ignore_ascii_case(&'z') => {
            if key.modifiers.shift {
                Some(StandardShortcut::Redo)
            } else {
                Some(StandardShortcut::Undo)
            }
        }
        Key::Character(ch)
            if !is_apple && key.modifiers.control && ch.eq_ignore_ascii_case(&'y') =>
        {
            Some(StandardShortcut::Redo)
        }
        _ => None,
    }
}

struct TextInputInstance {
    payload: TextInputPayload,
    engine: TextEngine,
    editor: SingleLineEditor,
    style: Option<TextInputStyle>,
    undo: Undoer,
    focused: bool,
    dragging: bool,
    scroll: f32,
    click: Option<ClickState>,
    accepted_value: Option<String>,
    accepted_revision: Option<u64>,
    pending_acknowledgements: VecDeque<String>,
    pending_external: Option<(String, u64)>,
    pending_value: Option<String>,
    pending_deadline: Option<Instant>,
    pending_generation: u64,
    last_cursor_param: Option<u64>,
    last_selected_param: Option<String>,
}

impl TextInputInstance {
    fn new(payload: TextInputPayload, engine: TextEngine) -> Self {
        Self {
            payload,
            engine,
            editor: SingleLineEditor::new(""),
            style: None,
            undo: Undoer::default(),
            focused: false,
            dragging: false,
            scroll: 0.0,
            click: None,
            accepted_value: None,
            accepted_revision: None,
            pending_acknowledgements: VecDeque::new(),
            pending_external: None,
            pending_value: None,
            pending_deadline: None,
            pending_generation: 0,
            last_cursor_param: None,
            last_selected_param: None,
        }
    }

    fn edit_state(&self) -> EditState {
        EditState {
            selection: self.editor.selection(),
            text: self.editor.committed_text().into_string(),
        }
    }

    fn style(&self) -> Result<TextInputStyle, AvengerChartError> {
        self.style.clone().ok_or_else(|| {
            AvengerChartError::InternalError("TextInput has no environment style".to_string())
        })
    }

    fn config(style: &TextInputStyle) -> TextMeasurementConfig<'_> {
        TextMeasurementConfig {
            // SingleLineEditor substitutes its current buffer before shaping.
            text: "",
            font: &style.font,
            font_size: style.font_size,
            font_weight: style.font_weight,
            font_style: FontStyle::Normal,
            syntax_mode: TextSyntaxMode::Plain,
            params: empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        }
    }

    fn apply_action(&mut self, action: Action, now: Instant) -> Result<bool, AvengerChartError> {
        let style = self.style()?;
        let config = Self::config(&style);
        let before = self.edit_state();
        if self.editor.compose_range().is_none() {
            self.undo.feed(now, before);
        }
        let changed = self
            .editor
            .apply(action, &self.engine, &config)
            .map_err(text_error)?;
        if self.editor.compose_range().is_none() {
            self.undo.feed(now, self.edit_state());
        }
        Ok(changed)
    }

    fn restore_edit_state(&mut self, state: EditState) {
        self.editor
            .restore_committed_state(state.text, state.selection);
    }

    fn schedule_commit(&mut self, ctx: &NativeWidgetCtx) {
        if self.payload.commit != TextCommit::OnChange {
            return;
        }
        let value = self.editor.committed_text().into_string();
        if self.payload.debounce_ms == 0 {
            self.commit_value(value, ctx);
            return;
        }
        self.pending_generation = self.pending_generation.wrapping_add(1).max(1);
        let deadline = ctx.now() + Duration::from_millis(self.payload.debounce_ms);
        self.pending_value = Some(value);
        self.pending_deadline = Some(deadline);
        ctx.request_wakeup(COMMIT_WAKE, deadline, self.pending_generation);
    }

    fn flush_commit(&mut self, ctx: &NativeWidgetCtx) {
        if self.pending_deadline.take().is_some() {
            ctx.cancel_wakeup(COMMIT_WAKE);
        }
        self.pending_value = None;
        self.commit_value(self.editor.committed_text().into_string(), ctx);
    }

    fn cancel_commit(&mut self, ctx: &NativeWidgetCtx) {
        if self.pending_deadline.take().is_some() {
            ctx.cancel_wakeup(COMMIT_WAKE);
        }
        self.pending_value = None;
        self.pending_generation = self.pending_generation.wrapping_add(1).max(1);
    }

    fn commit_value(&mut self, value: String, ctx: &NativeWidgetCtx) {
        if self.accepted_value.as_ref() == Some(&value)
            || self
                .pending_acknowledgements
                .iter()
                .any(|pending| pending == &value)
        {
            return;
        }
        self.pending_acknowledgements.push_back(value.clone());
        ctx.assign_param(root_assignment(
            self.payload.value_param.clone(),
            ScalarValue::Utf8(Some(value)),
        ));
        ctx.request_evaluation(NativeWidgetEvaluationIntent::Exact);
    }

    fn publish_editing_state(&mut self, ctx: &NativeWidgetCtx) {
        let committed = self.editor.committed_text().into_string();
        let selection = committed_selection(&self.editor);
        if let Some(name) = self.payload.cursor_param.as_ref() {
            let cursor = committed[..selection.head.index.min(committed.len())]
                .graphemes(true)
                .count() as u64;
            if self.last_cursor_param != Some(cursor) {
                self.last_cursor_param = Some(cursor);
                ctx.assign_param(root_assignment(
                    name.clone(),
                    ScalarValue::UInt64(Some(cursor)),
                ));
                ctx.request_evaluation(NativeWidgetEvaluationIntent::Exact);
            }
        }
        if let Some(name) = self.payload.selected_text_param.as_ref() {
            let range = normalized_selection(selection);
            let selected = committed.get(range).unwrap_or_default().to_string();
            if self.last_selected_param.as_ref() != Some(&selected) {
                self.last_selected_param = Some(selected.clone());
                ctx.assign_param(root_assignment(
                    name.clone(),
                    ScalarValue::Utf8(Some(selected)),
                ));
                ctx.request_evaluation(NativeWidgetEvaluationIntent::Exact);
            }
        }
    }

    fn update_scroll_and_focus(&mut self, ctx: &NativeWidgetCtx) -> Result<(), AvengerChartError> {
        let style = self.style()?;
        let config = Self::config(&style);
        let selection = self.editor.selection();
        let line = self
            .editor
            .shape_line(&self.engine, &config)
            .map_err(text_error)?;
        let caret = cursor_rect_for_offset(line, selection.head.index, selection.head.affinity);
        let width = (style.frame_size[0] - 2.0 * style.inset).max(0.0);
        if caret.x - self.scroll > width - SCROLL_PAD {
            self.scroll = caret.x - width + SCROLL_PAD;
        } else if caret.x - self.scroll < SCROLL_PAD {
            self.scroll = (caret.x - SCROLL_PAD).max(0.0);
        }
        self.scroll = self.scroll.clamp(0.0, (line.bounds.width - width).max(0.0));
        if self.focused {
            let text_top = (style.frame_size[1] - line.bounds.height) * 0.5;
            let rect = LogicalRect::new(
                style.inset + caret.x - self.scroll,
                text_top + caret.y,
                style.caret_width.max(1.0),
                caret.height,
            );
            ctx.focus(rect, self.editor.selected_text());
        }
        Ok(())
    }

    fn lose_focus(&mut self, ctx: &NativeWidgetCtx, commit: bool) {
        if !self.focused {
            return;
        }
        if commit {
            self.flush_commit(ctx);
        }
        self.focused = false;
        self.dragging = false;
        ctx.blur();
        ctx.mark_scene_dirty();
    }

    fn apply_external(&mut self, value: String, revision: u64, ctx: &NativeWidgetCtx) {
        if self.editor.compose_range().is_some() {
            self.pending_external = Some((value, revision));
            return;
        }
        self.accepted_value = Some(value.clone());
        self.accepted_revision = Some(revision);
        self.pending_acknowledgements.clear();
        if self.editor.committed_text().to_string() == value {
            if self.pending_deadline.is_some() {
                self.cancel_commit(ctx);
            }
            return;
        }
        self.cancel_commit(ctx);
        self.editor.replace_committed_text(value);
        self.undo.clear(ctx.now(), self.edit_state());
        self.scroll = 0.0;
        ctx.mark_scene_dirty();
    }

    fn apply_pending_external(&mut self, ctx: &NativeWidgetCtx) {
        if self.editor.compose_range().is_none()
            && let Some((value, revision)) = self.pending_external.take()
        {
            self.apply_external(value, revision, ctx);
        }
    }

    fn handle_key(
        &mut self,
        key: &SceneKeyPressEvent,
        ctx: &NativeWidgetCtx,
    ) -> Result<bool, AvengerChartError> {
        let is_apple = cfg!(target_vendor = "apple");
        let before = self.editor.committed_text().into_string();
        let changed = if let Some(action) = text_shortcut(key, is_apple) {
            self.apply_action(action, ctx.now())?
        } else if let Some(shortcut) = standard_shortcut(key, is_apple) {
            match shortcut {
                StandardShortcut::SelectAll => self.apply_action(Action::SelectAll, ctx.now())?,
                StandardShortcut::Undo | StandardShortcut::Redo => {
                    let current = self.edit_state();
                    let restored = match shortcut {
                        StandardShortcut::Undo => self.undo.undo(current),
                        StandardShortcut::Redo => self.undo.redo(current),
                        StandardShortcut::SelectAll => unreachable!(),
                    };
                    if let Some(restored) = restored {
                        self.restore_edit_state(restored);
                        self.schedule_commit(ctx);
                        true
                    } else {
                        false
                    }
                }
            }
        } else if matches!(key.key, Key::Named(NamedKey::Enter)) {
            self.flush_commit(ctx);
            if self.payload.commit == TextCommit::OnEnterOrBlur {
                self.lose_focus(ctx, false);
            }
            return Ok(true);
        } else {
            let command = if is_apple {
                key.modifiers.meta
            } else {
                key.modifiers.control
            };
            let action = (!command && !key.modifiers.control && !key.modifiers.alt)
                .then(|| {
                    key.text
                        .as_ref()
                        .map(|text| Action::InsertText(text.to_string()))
                })
                .flatten();
            action
                .map(|action| self.apply_action(action, ctx.now()))
                .transpose()?
                .unwrap_or(false)
        };
        if changed && self.editor.committed_text().to_string() != before {
            self.schedule_commit(ctx);
        }
        Ok(changed)
    }

    fn handle_clipboard(
        &mut self,
        event: &ClipboardEvent,
        ctx: &NativeWidgetCtx,
    ) -> Result<bool, AvengerChartError> {
        match event {
            ClipboardEvent::Copy => {
                ctx.write_clipboard(self.editor.selected_text());
                Ok(false)
            }
            ClipboardEvent::Cut => {
                let selected = self.editor.selected_text().to_string();
                ctx.write_clipboard(selected);
                let changed = self.apply_action(Action::Backspace, ctx.now())?;
                if changed {
                    self.schedule_commit(ctx);
                }
                Ok(changed)
            }
            ClipboardEvent::Paste(text) => {
                let changed = self.apply_action(Action::InsertText(text.to_string()), ctx.now())?;
                if changed {
                    self.schedule_commit(ctx);
                }
                Ok(changed)
            }
        }
    }

    fn handle_ime(
        &mut self,
        event: &ImeEvent,
        ctx: &NativeWidgetCtx,
    ) -> Result<bool, AvengerChartError> {
        let before = self.editor.committed_text().into_string();
        let changed = match event {
            ImeEvent::Enabled => false,
            ImeEvent::Preedit { text, cursor } => self.apply_action(
                Action::Preedit {
                    text: text.to_string(),
                    cursor: *cursor,
                },
                ctx.now(),
            )?,
            ImeEvent::Commit(text) => {
                self.apply_action(Action::Commit(text.to_string()), ctx.now())?
            }
            ImeEvent::Disabled => {
                let first = self.apply_action(
                    Action::Preedit {
                        text: String::new(),
                        cursor: None,
                    },
                    ctx.now(),
                )?;
                let second = self.apply_action(Action::Commit(String::new()), ctx.now())?;
                first || second
            }
        };
        if self.editor.compose_range().is_none() {
            if self.editor.committed_text().to_string() != before {
                self.schedule_commit(ctx);
            }
            self.apply_pending_external(ctx);
        }
        Ok(changed)
    }

    fn resolve_style(
        environment: &NativeWidgetEnvironment,
        ctx: &NativeWidgetCtx,
    ) -> Result<TextInputStyle, AvengerChartError> {
        let box_theme = ctx.part_theme("box", "rect")?;
        let text_theme = ctx.part_theme("text", "text")?;
        let placeholder = ctx.part_theme("placeholder", "text")?;
        let selection = ctx.part_theme("selection", "rect")?;
        let caret = ctx.part_theme("caret", "rule")?;
        let preedit = ctx.part_theme("preedit", "rule")?;
        let focus = ctx.part_theme("focus-ring", "rect")?;
        let font_weight = text_theme
            .string(WidgetStyleProperty::FontWeight)?
            .as_deref()
            .map(parse_font_weight)
            .unwrap_or_default();
        Ok(TextInputStyle {
            frame_size: environment.frame_size,
            inset: required_length(&box_theme, WidgetStyleProperty::InputInlineInset)?,
            box_fill: required_color(&box_theme, WidgetStyleProperty::Fill)?,
            box_stroke: required_color(&box_theme, WidgetStyleProperty::Stroke)?,
            border_width: required_length(&box_theme, WidgetStyleProperty::StrokeWidth)?,
            radius: required_length(&box_theme, WidgetStyleProperty::CornerRadius)?,
            text_color: required_color(&text_theme, WidgetStyleProperty::Fill)?,
            placeholder_color: required_color(&placeholder, WidgetStyleProperty::Fill)?,
            selection_color: required_color(&selection, WidgetStyleProperty::Fill)?,
            selection_opacity: selection
                .number(WidgetStyleProperty::Opacity)?
                .unwrap_or(1.0),
            caret_color: required_color(&caret, WidgetStyleProperty::Stroke)?,
            caret_width: required_length(&caret, WidgetStyleProperty::InputCaretWidth)?,
            preedit_color: required_color(&preedit, WidgetStyleProperty::Stroke)?,
            preedit_width: required_length(&preedit, WidgetStyleProperty::StrokeWidth)?,
            focus_color: required_color(&focus, WidgetStyleProperty::Stroke)?,
            focus_width: required_length(&focus, WidgetStyleProperty::FocusRingWidth)?,
            focus_gap: required_length(&focus, WidgetStyleProperty::FocusGap)?,
            font: text_theme
                .string(WidgetStyleProperty::FontFamily)?
                .unwrap_or_else(|| "sans-serif".to_string()),
            font_size: required_length(&text_theme, WidgetStyleProperty::FontSize)?,
            font_weight,
        })
    }

    fn build_scene(&mut self) -> Result<NativeWidgetScene, AvengerChartError> {
        let style = self.style()?;
        let config = Self::config(&style);
        let line = self
            .editor
            .shape_line(&self.engine, &config)
            .map_err(text_error)?
            .clone();
        let text_top = (style.frame_size[1] - line.bounds.height) * 0.5;
        let text_x = style.inset - self.scroll;
        // Let each renderer center its own resolved font metrics. Computing an
        // alphabetic baseline with this instance's shaping engine can drift
        // vertically when the host renderer resolves a different fallback face.
        let text_middle = style.frame_size[1] * 0.5;
        let selected_range = self.editor.normalized_selection();
        let selection = if selected_range.is_empty() {
            Vec::new()
        } else {
            selection_rects(&line, selected_range)
        };
        let compose = self.editor.compose_range();
        let preedit = compose
            .clone()
            .map(|range| selection_rects(&line, range))
            .unwrap_or_default();
        let caret = cursor_rect_for_offset(
            &line,
            self.editor.selection().head.index,
            self.editor.selection().head.affinity,
        );
        let mut selection_color = style.selection_color;
        selection_color[3] *= style.selection_opacity;
        let show_text = !self.editor.text().is_empty() || compose.is_some();
        let show_placeholder = self.editor.text().is_empty() && compose.is_none();

        NativeWidgetScene::try_from_iter([
            (
                "box".to_string(),
                SceneMark::Rect(SceneRectMark {
                    x: 0.0.into(),
                    y: 0.0.into(),
                    width: Some(style.frame_size[0].into()),
                    height: Some(style.frame_size[1].into()),
                    fill: ColorOrGradient::Color(style.box_fill).into(),
                    stroke: ColorOrGradient::Color(style.box_stroke).into(),
                    stroke_width: style.border_width.into(),
                    corner_radius: style.radius.into(),
                    ..Default::default()
                }),
            ),
            (
                "selection".to_string(),
                SceneMark::Rect(rects_mark(&selection, text_x, text_top, selection_color)),
            ),
            (
                "text".to_string(),
                SceneMark::Text(std::sync::Arc::new(SceneTextMark {
                    text: if show_text {
                        self.editor.text().to_string()
                    } else {
                        String::new()
                    }
                    .into(),
                    x: text_x.into(),
                    y: text_middle.into(),
                    color: ColorOrGradient::Color(style.text_color).into(),
                    font: style.font.clone().into(),
                    font_size: style.font_size.into(),
                    font_weight: style.font_weight.into(),
                    align: TextAlign::Left.into(),
                    baseline: TextBaseline::Middle.into(),
                    text_syntax: TextSyntaxMode::Plain,
                    limit: (style.frame_size[0] - style.inset).max(0.0).into(),
                    ..Default::default()
                })),
            ),
            (
                "placeholder".to_string(),
                SceneMark::Text(std::sync::Arc::new(SceneTextMark {
                    text: if show_placeholder {
                        self.payload.placeholder.clone()
                    } else {
                        String::new()
                    }
                    .into(),
                    x: style.inset.into(),
                    y: text_middle.into(),
                    color: ColorOrGradient::Color(style.placeholder_color).into(),
                    font: style.font.clone().into(),
                    font_size: style.font_size.into(),
                    font_weight: style.font_weight.into(),
                    align: TextAlign::Left.into(),
                    baseline: TextBaseline::Middle.into(),
                    text_syntax: TextSyntaxMode::Plain,
                    limit: (style.frame_size[0] - style.inset).max(0.0).into(),
                    ..Default::default()
                })),
            ),
            (
                "preedit".to_string(),
                SceneMark::Rule(underline_mark(
                    &preedit,
                    text_x,
                    text_top,
                    style.preedit_color,
                    style.preedit_width,
                )),
            ),
            (
                "caret".to_string(),
                SceneMark::Rule(SceneRuleMark {
                    len: u32::from(self.focused && self.editor.show_cursor()),
                    x: (text_x + caret.x).into(),
                    x2: (text_x + caret.x).into(),
                    y: (text_top + caret.y).into(),
                    y2: (text_top + caret.y + caret.height).into(),
                    stroke: ColorOrGradient::Color(style.caret_color).into(),
                    stroke_width: style.caret_width.into(),
                    stroke_cap: StrokeCap::Butt.into(),
                    ..Default::default()
                }),
            ),
            (
                "focus-ring".to_string(),
                SceneMark::Rect(SceneRectMark {
                    x: (-style.focus_gap).into(),
                    y: (-style.focus_gap).into(),
                    width: Some((style.frame_size[0] + 2.0 * style.focus_gap).into()),
                    height: Some((style.frame_size[1] + 2.0 * style.focus_gap).into()),
                    fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                    stroke: ColorOrGradient::Color(if self.focused {
                        style.focus_color
                    } else {
                        [0.0, 0.0, 0.0, 0.0]
                    })
                    .into(),
                    stroke_width: style.focus_width.into(),
                    corner_radius: (style.radius + style.focus_gap).into(),
                    ..Default::default()
                }),
            ),
        ])
    }
}

impl NativeWidgetInstance for TextInputInstance {
    fn on_state_sync(
        &mut self,
        state: NativeWidgetStateSnapshot<'_>,
        ctx: &mut NativeWidgetCtx,
    ) -> Result<(), AvengerChartError> {
        let value = match state.get(&self.payload.value_param) {
            Some(ScalarValue::Utf8(Some(value))) => value.clone(),
            value => {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "TextInput value param '{}' must be non-null Utf8, got {value:?}",
                    self.payload.value_param
                )));
            }
        };
        let revision = state.revision(&self.payload.value_param);
        if self.accepted_revision == Some(revision) {
            return Ok(());
        }
        if let Some(index) = self
            .pending_acknowledgements
            .iter()
            .position(|pending| pending == &value)
        {
            self.pending_acknowledgements.drain(..=index);
            self.accepted_value = Some(value);
            self.accepted_revision = Some(revision);
            return Ok(());
        }
        let initial = self.accepted_value.is_none();
        if initial {
            self.accepted_value = Some(value.clone());
            self.accepted_revision = Some(revision);
            self.editor = SingleLineEditor::new(value);
            self.undo.clear(ctx.now(), self.edit_state());
            ctx.mark_scene_dirty();
        } else {
            self.apply_external(value, revision, ctx);
        }
        Ok(())
    }

    fn on_environment_sync(
        &mut self,
        environment: &NativeWidgetEnvironment,
        ctx: &mut NativeWidgetCtx,
    ) -> Result<(), AvengerChartError> {
        let style = Self::resolve_style(environment, ctx)?;
        if self.style.as_ref() != Some(&style) {
            self.style = Some(style);
            self.editor.mark_layout_dirty();
            ctx.mark_scene_dirty();
            ctx.mark_index_dirty();
        }
        if let Some(deadline) = self.pending_deadline {
            ctx.request_wakeup(COMMIT_WAKE, deadline, self.pending_generation);
        }
        self.update_scroll_and_focus(ctx)?;
        Ok(())
    }

    fn on_event(
        &mut self,
        event: &NativeWidgetEvent,
        ctx: &mut NativeWidgetCtx,
    ) -> Result<(), AvengerChartError> {
        self.apply_pending_external(ctx);
        let mut changed = false;
        match &event.event {
            SceneGraphEvent::MouseDown(mouse) if mouse.button == MouseButton::Left => {
                let point = event.current.unwrap_or([0.0, 0.0]);
                let inside_frame = point[0] >= 0.0
                    && point[1] >= 0.0
                    && point[0] <= event.frame_size[0]
                    && point[1] <= event.frame_size[1];
                if event.hit_part.is_none() && !inside_frame {
                    self.lose_focus(ctx, true);
                    return Ok(());
                }
                if !self.focused {
                    self.focused = true;
                    ctx.mark_scene_dirty();
                }
                self.dragging = true;
                let count = next_click_count(self.click, ctx.now(), point);
                self.click = Some(ClickState {
                    at: ctx.now(),
                    point,
                    count,
                });
                let style = self.style.as_ref().expect("event follows environment sync");
                let x = point[0] - style.inset + self.scroll;
                changed = if count == 1 && mouse.modifiers.shift {
                    let style = self.style()?;
                    let config = Self::config(&style);
                    let line = self
                        .editor
                        .shape_line(&self.engine, &config)
                        .map_err(text_error)?;
                    let (index, affinity) = avenger_text::text_edit::byte_offset_for_x(line, x);
                    let mut selection = self.editor.selection();
                    selection.head = Cursor::new(index, affinity);
                    self.editor.set_selection(selection)
                } else {
                    self.apply_action(
                        match count {
                            1 => Action::Click { x },
                            2 => Action::DoubleClick { x },
                            _ => Action::TripleClick,
                        },
                        ctx.now(),
                    )?
                };
                ctx.consume();
            }
            SceneGraphEvent::CursorMoved(_) if self.focused && self.dragging => {
                if let Some(point) = event.current {
                    let style = self.style.as_ref().expect("event follows environment sync");
                    changed = self.apply_action(
                        Action::Drag {
                            x: point[0] - style.inset + self.scroll,
                        },
                        ctx.now(),
                    )?;
                    ctx.consume();
                }
            }
            SceneGraphEvent::MouseUp(mouse)
                if mouse.button == MouseButton::Left && self.focused && self.dragging =>
            {
                self.dragging = false;
                ctx.consume();
            }
            SceneGraphEvent::KeyPress(key) if self.focused => {
                changed = self.handle_key(key, ctx)?;
                ctx.consume();
            }
            SceneGraphEvent::Clipboard(clipboard) if self.focused => {
                changed = self.handle_clipboard(clipboard, ctx)?;
                ctx.consume();
            }
            SceneGraphEvent::Ime(ime) if self.focused => {
                changed = self.handle_ime(ime, ctx)?;
                ctx.consume();
            }
            SceneGraphEvent::RuntimeWake(wake)
                if ctx.accepts_wakeup_generation(wake, COMMIT_WAKE, self.pending_generation) =>
            {
                if let Some(deadline) = self.pending_deadline {
                    if ctx.now() < deadline {
                        ctx.request_wakeup(COMMIT_WAKE, deadline, self.pending_generation);
                    } else {
                        self.pending_deadline = None;
                        if let Some(value) = self.pending_value.take() {
                            self.commit_value(value, ctx);
                        }
                    }
                }
                ctx.consume();
            }
            SceneGraphEvent::WindowFocused(false) => self.lose_focus(ctx, true),
            SceneGraphEvent::MouseEnter(_) => ctx.set_cursor(ctx.text_cursor()),
            _ => {}
        }
        if changed {
            ctx.mark_scene_dirty();
        }
        if self.focused {
            self.update_scroll_and_focus(ctx)?;
        }
        self.publish_editing_state(ctx);
        Ok(())
    }

    fn scene(
        &mut self,
        _environment: &NativeWidgetEnvironment,
        _ctx: &mut NativeWidgetCtx,
    ) -> Result<NativeWidgetScene, AvengerChartError> {
        self.build_scene()
    }

    fn on_deactivate(&mut self, ctx: &mut NativeWidgetCtx) -> Result<(), AvengerChartError> {
        self.lose_focus(ctx, true);
        Ok(())
    }

    fn on_session_detach(&mut self, _ctx: &mut NativeWidgetCtx) -> Result<(), AvengerChartError> {
        Ok(())
    }

    fn on_unmount(&mut self, ctx: &mut NativeWidgetCtx) -> Result<(), AvengerChartError> {
        self.cancel_commit(ctx);
        self.lose_focus(ctx, false);
        Ok(())
    }
}

fn root_assignment(name: String, value: ScalarValue) -> ScopedParamAssignment {
    ScopedParamAssignment {
        name,
        owner_path: Vec::new(),
        value,
        replace_scoped_values: false,
    }
}

fn next_click_count(previous: Option<ClickState>, now: Instant, point: [f32; 2]) -> u8 {
    let Some(previous) = previous else { return 1 };
    let close_in_time = now.saturating_duration_since(previous.at) <= CLICK_GAP;
    let close_in_space =
        (point[0] - previous.point[0]).hypot(point[1] - previous.point[1]) < CLICK_DISTANCE;
    if !close_in_time || !close_in_space {
        1
    } else {
        match previous.count {
            1 => 2,
            2 => 3,
            _ => 2,
        }
    }
}

fn committed_selection(editor: &SingleLineEditor) -> SelectionState {
    let Some(compose) = editor.compose_range() else {
        return editor.selection();
    };
    let map = |mut cursor: Cursor| {
        cursor.index = if cursor.index <= compose.start {
            cursor.index
        } else if cursor.index >= compose.end {
            cursor.index - (compose.end - compose.start)
        } else {
            compose.start
        };
        cursor
    };
    let selection = editor.selection();
    SelectionState {
        anchor: map(selection.anchor),
        head: map(selection.head),
        granularity: selection.granularity,
    }
}

fn normalized_selection(selection: SelectionState) -> Range<usize> {
    selection.anchor.index.min(selection.head.index)
        ..selection.anchor.index.max(selection.head.index)
}

fn required_length(
    theme: &avenger_chart::prelude::NativeWidgetPartTheme<'_>,
    property: WidgetStyleProperty,
) -> Result<f32, AvengerChartError> {
    theme.length(property)?.ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "TextInput part is missing required '{}' length",
            property.name()
        ))
    })
}

fn required_color(
    theme: &avenger_chart::prelude::NativeWidgetPartTheme<'_>,
    property: WidgetStyleProperty,
) -> Result<[f32; 4], AvengerChartError> {
    theme.color(property)?.ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "TextInput part is missing required '{}' color",
            property.name()
        ))
    })
}

fn parse_font_weight(weight: &str) -> FontWeight {
    match weight {
        "bold" => FontWeight::Name(FontWeightNameSpec::Bold),
        "normal" => FontWeight::Name(FontWeightNameSpec::Normal),
        value => value
            .parse::<f32>()
            .map(FontWeight::Number)
            .unwrap_or_default(),
    }
}

fn rects_mark(
    rects: &[avenger_text::text_edit::TextRect],
    x: f32,
    y: f32,
    color: [f32; 4],
) -> SceneRectMark {
    SceneRectMark {
        len: rects.len() as u32,
        x: ScalarOrArray::new_array(rects.iter().map(|rect| x + rect.x).collect()),
        y: ScalarOrArray::new_array(rects.iter().map(|rect| y + rect.y).collect()),
        width: Some(ScalarOrArray::new_array(
            rects.iter().map(|rect| rect.width).collect(),
        )),
        height: Some(ScalarOrArray::new_array(
            rects.iter().map(|rect| rect.height).collect(),
        )),
        fill: ColorOrGradient::Color(color).into(),
        stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
        stroke_width: 0.0.into(),
        ..Default::default()
    }
}

fn underline_mark(
    rects: &[avenger_text::text_edit::TextRect],
    x: f32,
    y: f32,
    color: [f32; 4],
    width: f32,
) -> SceneRuleMark {
    let y_values = rects
        .iter()
        .map(|rect| y + rect.y + rect.height)
        .collect::<Vec<_>>();
    SceneRuleMark {
        len: rects.len() as u32,
        x: ScalarOrArray::new_array(rects.iter().map(|rect| x + rect.x).collect()),
        x2: ScalarOrArray::new_array(rects.iter().map(|rect| x + rect.x + rect.width).collect()),
        y: ScalarOrArray::new_array(y_values.clone()),
        y2: ScalarOrArray::new_array(y_values),
        stroke: ColorOrGradient::Color(color).into(),
        stroke_width: width.into(),
        stroke_cap: StrokeCap::Butt.into(),
        ..Default::default()
    }
}

fn text_error(error: avenger_text::error::AvengerTextError) -> AvengerChartError {
    AvengerChartError::InternalError(format!("TextInput shaping failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_eventstream::scene::ModifiersState;

    #[test]
    fn click_cycle_is_single_double_triple_double() {
        let start = Instant::now();
        let point = [4.0, 5.0];
        let one = ClickState {
            at: start,
            point,
            count: 1,
        };
        assert_eq!(
            next_click_count(Some(one), start + Duration::from_millis(10), point),
            2
        );
        let two = ClickState {
            at: start,
            point,
            count: 2,
        };
        assert_eq!(
            next_click_count(Some(two), start + Duration::from_millis(10), point),
            3
        );
        let three = ClickState {
            at: start,
            point,
            count: 3,
        };
        assert_eq!(
            next_click_count(Some(three), start + Duration::from_millis(10), point),
            2
        );
    }

    #[test]
    fn undo_batches_unsettled_typing_into_one_restore() {
        let now = Instant::now();
        let initial = EditState {
            selection: SelectionState::default(),
            text: String::new(),
        };
        let typed = EditState {
            selection: SelectionState::default(),
            text: "abc".to_string(),
        };
        let mut undo = Undoer::default();
        undo.feed(now, initial.clone());
        undo.feed(now + Duration::from_millis(20), typed.clone());
        assert_eq!(undo.undo(typed), Some(initial));
    }

    #[test]
    fn new_edit_after_undo_clears_redo_history() {
        let now = Instant::now();
        let state = |text: &str| EditState {
            selection: SelectionState::default(),
            text: text.to_string(),
        };
        let mut undo = Undoer::default();
        undo.feed(now, state(""));
        undo.feed(now + Duration::from_millis(20), state("old future"));
        assert_eq!(undo.undo(state("old future")), Some(state("")));

        undo.feed(now + Duration::from_millis(40), state("new future"));
        assert_eq!(undo.redo(state("new future")), None);
    }

    #[test]
    fn keybinding_tables_cover_apple_and_non_apple_variants() {
        let event = |key, modifiers| SceneKeyPressEvent {
            position: [0.0, 0.0],
            key,
            text: None,
            mark_instance: None,
            modifiers,
        };
        assert!(matches!(
            text_shortcut(
                &event(
                    Key::Named(NamedKey::ArrowLeft),
                    ModifiersState {
                        alt: true,
                        ..Default::default()
                    }
                ),
                true
            ),
            Some(Action::Motion {
                motion: Motion::WordLeft,
                extend: false
            })
        ));
        assert!(matches!(
            text_shortcut(
                &event(
                    Key::Named(NamedKey::ArrowLeft),
                    ModifiersState {
                        meta: true,
                        shift: true,
                        ..Default::default()
                    }
                ),
                true
            ),
            Some(Action::Motion {
                motion: Motion::Start,
                extend: true
            })
        ));
        assert!(matches!(
            text_shortcut(
                &event(
                    Key::Named(NamedKey::Delete),
                    ModifiersState {
                        control: true,
                        ..Default::default()
                    }
                ),
                false
            ),
            Some(Action::DeleteWordForward)
        ));
        assert_eq!(
            standard_shortcut(
                &event(
                    Key::Character('z'),
                    ModifiersState {
                        control: true,
                        shift: true,
                        ..Default::default()
                    }
                ),
                false
            ),
            Some(StandardShortcut::Redo)
        );
    }
}
