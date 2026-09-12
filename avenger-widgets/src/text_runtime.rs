use crate::{
    Rect, TextCancelReason, TextCommitPolicy, TextCommitReason, TextInput, TextShortcuts,
    WidgetAction, WidgetError, WidgetId, WidgetRuntime, WidgetSpec, WidgetTarget, WidgetUpdate,
};
use crate::{
    frame::{intersection, visible},
    paint::{clip, rect, rule, text},
    runtime::GestureKind,
    text_input::{EditClass, History, Snapshot, TextLayout, TextState, debounce_config},
};
use avenger_color::ColorOrGradient;
use avenger_common::time::{Duration, Instant};
use avenger_eventstream::{
    runtime::{
        DebouncedCommit, InputSession, LogicalRect, RuntimeHostCommand as Command, RuntimeWakeKey,
    },
    scene::{ModifiersState, SceneGraphEvent as Event},
    window::{ClipboardEvent, ElementState, ImeEvent, Key, NamedKey, TextInputEvent},
};
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    rect::SceneRectMark,
};
use avenger_text::text_edit::{
    Action, Motion, SelectionState, cursor_rect_for_offset, normalize_single_line, selection_rects,
};

fn wake_key(namespace: u64, epoch: u64, purpose: &str) -> RuntimeWakeKey {
    RuntimeWakeKey::new(format!("avenger-widgets-{namespace}"), epoch, purpose)
}
fn cancel_pending(state: &mut TextState, key: &RuntimeWakeKey, update: &mut WidgetUpdate) {
    state.pending = false;
    update
        .status
        .commands
        .extend(state.debounce.cancel(key).commands);
}
fn commit(
    state: &mut TextState,
    id: &WidgetId,
    value: String,
    reason: TextCommitReason,
    update: &mut WidgetUpdate,
) {
    state.pending = false;
    state.baseline = value.clone();
    update.emit(id, WidgetAction::TextCommitted { value, reason });
}
fn changed(
    state: &mut TextState,
    spec: &mut TextInput,
    epoch: u64,
    namespace: u64,
    now: Instant,
    update: &mut WidgetUpdate,
) {
    let value = state.editor.committed_text().into_string();
    if value == spec.value {
        return;
    }
    spec.value = value.clone();
    state.pending = true;
    update.emit(
        &spec.options.id,
        WidgetAction::TextChanged {
            value: value.clone(),
        },
    );
    match &spec.policy {
        TextCommitPolicy::Immediate => commit(
            state,
            &spec.options.id,
            value,
            TextCommitReason::Immediate,
            update,
        ),
        TextCommitPolicy::Debounced(_) => {
            let result = state
                .debounce
                .submit(value, now, &wake_key(namespace, epoch, "commit"));
            update.status.commands.extend(result.commands);
            if let Some(value) = result.commit {
                commit(
                    state,
                    &spec.options.id,
                    value,
                    TextCommitReason::Immediate,
                    update,
                );
            }
        }
        TextCommitPolicy::OnEnterOrBlur => {}
    }
}
fn discard_composition(state: &mut TextState) {
    if let Some(before) = state.composition.take() {
        before.restore(&mut state.editor);
    }
    state.history.boundary();
}

impl WidgetRuntime {
    /// Query the focused host session. Adapters tag queued input with this identity.
    pub fn active_input_session(&self) -> Option<&InputSession> {
        self.focused.as_ref().and_then(|t| {
            self.controls
                .get(&t.widget)?
                .text
                .as_ref()?
                .session
                .as_ref()
        })
    }
    /// Query byte offsets at grapheme boundaries in the installed editor.
    pub fn text_selection(&self, id: impl Into<WidgetId>) -> Option<SelectionState> {
        self.controls
            .get(&id.into())?
            .text
            .as_ref()
            .map(|s| s.editor.selection())
    }
    /// Whether a text field currently displays uncommitted IME preedit.
    pub fn text_is_composing(&self, id: impl Into<WidgetId>) -> bool {
        self.controls
            .get(&id.into())
            .and_then(|c| c.text.as_ref())
            .is_some_and(|s| s.composition.is_some())
    }
    /// Set a selection without changing the caller's draft value.
    pub fn set_text_selection(
        &mut self,
        id: impl Into<WidgetId>,
        selection: SelectionState,
        now: Instant,
    ) -> Result<WidgetUpdate, WidgetError> {
        self.now = now;
        let id = id.into();
        let state = self
            .controls
            .get_mut(&id)
            .and_then(|c| c.text.as_mut())
            .ok_or_else(|| WidgetError::Invalid("selection requires a text input".into()))?;
        if state.composition.is_some() {
            return Err(WidgetError::Invalid(
                "selection cannot be replaced during composition".into(),
            ));
        }
        state.editor.set_selection(selection);
        state.history.boundary();
        let mut update = WidgetUpdate::default();
        update.status.rerender = true;
        self.reset_text_blink(&id, &mut update);
        self.layout_text(&mut update)?;
        self.publish_policy(&mut update);
        self.revision += 1;
        Ok(update)
    }
    /// Cancel composition and pending work, replace the draft, and retire queued input.
    /// The application supplies the same normalized value in its next description.
    pub fn reset_text(
        &mut self,
        id: impl Into<WidgetId>,
        value: impl Into<String>,
        now: Instant,
    ) -> Result<WidgetUpdate, WidgetError> {
        self.now = now;
        let id = id.into();
        let target = WidgetTarget::new(id.clone());
        if !matches!(
            self.controls.get(&id).map(|c| &c.spec),
            Some(WidgetSpec::TextInput(_))
        ) {
            return Err(WidgetError::Invalid("reset requires a text input".into()));
        }
        let mut update = WidgetUpdate::default();
        self.stop_text_drag(&target, &mut update);
        self.deactivate_text(&target, Some(TextCancelReason::Reset), &mut update);
        self.next_epoch += 1;
        let c = self.controls.get_mut(&id).unwrap();
        c.epoch = self.next_epoch;
        let WidgetSpec::TextInput(spec) = &mut c.spec else {
            unreachable!()
        };
        spec.value = normalize_single_line(&value.into());
        for event in &mut update.events {
            if let WidgetAction::TextCancelled { value, .. } = &mut event.action {
                *value = spec.value.clone();
            }
        }
        c.text = Some(Box::new(TextState::new(spec)));
        update.status.rebuild_geometry = true;
        if self.focused.as_ref() == Some(&target) {
            self.activate_text_session(&target, &mut update);
        }
        update.status.rerender = true;
        self.layout_text(&mut update)?;
        self.publish_policy(&mut update);
        self.revision += 1;
        Ok(update)
    }
    pub(crate) fn activate_text_session(
        &mut self,
        target: &WidgetTarget,
        update: &mut WidgetUpdate,
    ) {
        let Some(c) = self.controls.get_mut(&target.widget) else {
            return;
        };
        let Some(s) = &mut c.text else {
            return;
        };
        self.next_session += 1;
        s.session = Some(InputSession {
            owner: format!("avenger-widgets-{}-{}", self.namespace, c.epoch).into(),
            generation: self.next_session,
        });
        s.history.boundary();
        self.reset_text_blink(&target.widget, update);
    }
    pub(crate) fn deactivate_text(
        &mut self,
        target: &WidgetTarget,
        cancel: Option<TextCancelReason>,
        update: &mut WidgetUpdate,
    ) {
        let Some(c) = self.controls.get_mut(&target.widget) else {
            return;
        };
        let Some(s) = &mut c.text else {
            return;
        };
        let WidgetSpec::TextInput(spec) = &mut c.spec else {
            return;
        };
        let active = s.session.take().is_some();
        let had_pending = s.pending;
        let had_composition = s.composition.is_some();
        discard_composition(s);
        if let Some(value) = s.deferred.take() {
            s.editor.replace_committed_text(&value);
            s.baseline = value.clone();
            spec.value = value;
            s.history = History::default();
            cancel_pending(s, &wake_key(self.namespace, c.epoch, "commit"), update);
        }
        if let Some(reason) = cancel {
            cancel_pending(s, &wake_key(self.namespace, c.epoch, "commit"), update);
            if active || had_pending || had_composition {
                update.emit(
                    &target.widget,
                    WidgetAction::TextCancelled {
                        value: spec.value.clone(),
                        reason,
                    },
                );
            }
        } else {
            update.status.commands.extend(
                s.debounce
                    .flush(&wake_key(self.namespace, c.epoch, "commit"))
                    .commands,
            );
            if s.pending {
                commit(
                    s,
                    &target.widget,
                    spec.value.clone(),
                    TextCommitReason::Blur,
                    update,
                );
            }
        }
        s.drag_x = None;
        s.last_click = None;
        s.blink_generation += 1;
        s.scroll_generation += 1;
        for purpose in ["blink", "scroll"] {
            update.status.commands.push(Command::CancelWakeup {
                key: wake_key(self.namespace, c.epoch, purpose),
            });
        }
        if active {
            update.status.rerender = true;
        }
    }
    pub(crate) fn reconcile_text(&mut self, new: &WidgetSpec, update: &mut WidgetUpdate) {
        let WidgetSpec::TextInput(new) = new else {
            return;
        };
        let id = &new.options.id;
        let Some(c) = self.controls.get(id) else {
            return;
        };
        let WidgetSpec::TextInput(old) = &c.spec else {
            return;
        };
        let unavailable = !new.options.enabled || old.read_only != new.read_only;
        let policy_changed = old.policy != new.policy;
        let external = old.value != new.value;
        if unavailable {
            self.deactivate_text(
                &WidgetTarget::new(id.clone()),
                Some(TextCancelReason::Disabled),
                update,
            );
        }
        let c = self.controls.get_mut(id).unwrap();
        let s = c.text.as_mut().unwrap();
        if policy_changed {
            cancel_pending(s, &wake_key(self.namespace, c.epoch, "commit"), update);
            s.debounce = DebouncedCommit::new(debounce_config(&new.policy));
        }
        if external {
            if s.composition.is_some() {
                s.deferred = Some(new.value.clone());
            } else {
                s.editor.replace_committed_text(&new.value);
                s.baseline = new.value.clone();
                s.history = History::default();
                cancel_pending(s, &wake_key(self.namespace, c.epoch, "commit"), update);
                s.drag_x = None;
            }
        }
        if external && s.composition.is_none() {
            s.session = None;
            s.blink_generation += 1;
            self.stop_text_drag(&WidgetTarget::new(id.clone()), update);
        }
    }
    pub(crate) fn publish_text_host(&self, update: &mut WidgetUpdate) {
        let active = self
            .focused
            .as_ref()
            .and_then(|t| self.controls.get(&t.widget));
        let session = active.and_then(|c| c.text.as_ref()?.session.clone());
        let editable = active.is_some_and(
            |c| matches!(&c.spec,WidgetSpec::TextInput(t) if t.options.enabled&&!t.read_only),
        );
        update
            .status
            .commands
            .push(Command::SetInputSession { session });
        update
            .status
            .commands
            .push(Command::SetImeAllowed { allowed: editable });
        let state = active.and_then(|c| c.text.as_ref());
        update.status.commands.push(Command::SetClipboardPayload {
            text: state.map_or_else(String::new, |s| s.editor.selected_text().into()),
        });
        let caret = state.and_then(|s| {
            s.layout.as_ref().and_then(|l| {
                if !visible(l.clip) {
                    return None;
                }
                let p = s.editor.selection().head;
                let caret = cursor_rect_for_offset(&l.line, p.index, p.affinity);
                let x = (l.origin[0] + caret.x).clamp(l.clip.x, l.clip.x + l.clip.width);
                let y = (l.origin[1] + caret.y).clamp(l.clip.y, l.clip.y + l.clip.height);
                LogicalRect::new(
                    x,
                    y,
                    1.0_f32.min(l.clip.width),
                    (caret.height).min((l.clip.y + l.clip.height - y).max(0.0)),
                )
            })
        });
        update.status.commands.push(Command::SetImeCursorArea {
            rect: if editable { caret } else { None },
        });
    }
    pub(crate) fn layout_text(&mut self, _update: &mut WidgetUpdate) -> Result<(), WidgetError> {
        let Some(engine) = &self.engine else {
            return Ok(());
        };
        for (id, c) in &mut self.controls {
            let WidgetSpec::TextInput(spec) = &c.spec else {
                continue;
            };
            let Some(region) = self
                .regions
                .iter()
                .find(|r| &r.target.widget == id && r.target.item.is_none())
            else {
                continue;
            };
            let s = c.text.as_mut().unwrap();
            let style = &self.theme.text_input;
            let typography = spec.text_style.as_ref().unwrap_or(&style.text);
            let line = s.editor.shape_line(engine, &typography.config(""))?.clone();
            let px = style.padding.min(region.rect.width / 2.0);
            let py = style.padding.min(region.rect.height / 2.0);
            let inner = Rect::new(
                region.rect.x + px,
                region.rect.y + py,
                (region.rect.width - 2.0 * px).max(0.0),
                (region.rect.height - 2.0 * py).max(0.0),
            );
            let cursor = s.editor.selection().head;
            let caret = cursor_rect_for_offset(&line, cursor.index, cursor.affinity);
            if s.drag_x.is_none() {
                if caret.x < s.scroll {
                    s.scroll = caret.x;
                } else if caret.x + 1.0 > s.scroll + inner.width {
                    s.scroll = caret.x + 1.0 - inner.width;
                }
            }
            s.scroll = s
                .scroll
                .clamp(0.0, (line.bounds.width + 1.0 - inner.width).max(0.0));
            let font = engine.font_metrics(&avenger_text::measurement::FontMetricsConfig {
                font: &typography.font,
                font_size: typography.size,
                font_weight: typography.weight,
                font_style: typography.style,
            })?;
            let baseline = region.rect.y + (region.rect.height - font.height) / 2.0 + font.ascent;
            let origin = [inner.x - s.scroll, baseline - line.baseline];
            s.layout = Some(TextLayout {
                inner,
                clip: intersection(inner, region.clip),
                origin,
                line,
            });
        }
        Ok(())
    }
    fn reset_text_blink(&mut self, id: &WidgetId, update: &mut WidgetUpdate) {
        let Some(c) = self.controls.get_mut(id) else {
            return;
        };
        let Some(s) = &mut c.text else {
            return;
        };
        s.caret_visible = true;
        s.blink_generation += 1;
        let key = wake_key(self.namespace, c.epoch, "blink");
        if s.session.is_some()
            && matches!(&c.spec,WidgetSpec::TextInput(t) if !t.read_only&&t.options.enabled)
            && s.editor.normalized_selection().is_empty()
            && s.editor.show_cursor()
        {
            update.status.commands.push(Command::RequestWakeup {
                key,
                deadline: self.now + Duration::from_millis(500),
                generation: s.blink_generation,
            });
        } else {
            update.status.commands.push(Command::CancelWakeup { key });
        }
    }
}

pub(crate) fn paint(
    runtime: &WidgetRuntime,
    id: &WidgetId,
    r: Rect,
    outer: Option<Rect>,
) -> SceneGroup {
    let c = &runtime.controls[id];
    let WidgetSpec::TextInput(spec) = &c.spec else {
        unreachable!()
    };
    let s = c.text.as_ref().unwrap();
    let style = &runtime.theme.text_input;
    let typography = spec.text_style.as_ref().unwrap_or(&style.text);
    let target = WidgetTarget::new(id.clone());
    let focused = runtime.focused.as_ref() == Some(&target);
    let enabled = spec.options.enabled;
    let colors = if spec.read_only && enabled {
        style.read_only
    } else {
        style
            .paint
            .resolve(enabled, runtime.hovered.as_ref() == Some(&target), false)
    };
    let mut group = SceneGroup {
        interactive: false,
        clip: outer.map_or(Clip::None, clip),
        ..Default::default()
    };
    if focused && runtime.focus_visible {
        let d = style.focus.gap + style.focus.width / 2.0;
        group.marks.push(rect(
            Rect::new(r.x - d, r.y - d, r.width + 2.0 * d, r.height + 2.0 * d),
            [0.0; 4],
            style.focus.color,
            style.focus.width,
            style.radius + d,
        ));
    }
    group.marks.push(rect(
        r,
        colors.fill,
        if spec.invalid && enabled {
            style.invalid_border
        } else {
            colors.border
        },
        style.border_width,
        style.radius,
    ));
    if let Some(l) = &s.layout {
        let mut content = SceneGroup {
            interactive: false,
            clip: clip(l.clip),
            ..Default::default()
        };
        if focused {
            for selection in selection_rects(&l.line, s.editor.normalized_selection()) {
                content.marks.push(rect(
                    Rect::new(
                        l.origin[0] + selection.x,
                        l.origin[1] + selection.y,
                        selection.width,
                        selection.height,
                    ),
                    style.selection,
                    [0.0; 4],
                    0.0,
                    0.0,
                ));
            }
        }
        let placeholder = s.editor.text().is_empty();
        content.marks.push(text(
            if placeholder {
                &spec.placeholder
            } else {
                s.editor.text()
            },
            l.origin[0],
            l.origin[1] + l.line.baseline,
            typography,
            if placeholder {
                style.placeholder
            } else {
                colors.foreground
            },
        ));
        if let Some(range) = s.editor.compose_range() {
            for segment in selection_rects(&l.line, range) {
                let y = l.origin[1] + segment.y + segment.height - 1.0;
                content.marks.push(rule(
                    [l.origin[0] + segment.x, y],
                    [l.origin[0] + segment.x + segment.width, y],
                    colors.foreground,
                    1.0,
                ));
            }
        }
        if focused
            && !spec.read_only
            && s.caret_visible
            && s.editor.show_cursor()
            && s.editor.normalized_selection().is_empty()
        {
            let cursor = s.editor.selection().head;
            let p = cursor_rect_for_offset(&l.line, cursor.index, cursor.affinity);
            content.marks.push(rect(
                Rect::new(l.origin[0] + p.x, l.origin[1] + p.y, 1.0, p.height),
                style.caret,
                [0.0; 4],
                0.0,
                0.0,
            ));
        }
        group.marks.push(content.into());
    }
    let region = runtime.regions.iter().find(|r| r.target == target).unwrap();
    let mut hit = SceneGroup {
        interactive: false,
        clip: clip(region.clip),
        ..Default::default()
    };
    hit.marks.push(
        SceneRectMark {
            name: region.name.clone(),
            interactive: true,
            clip: true,
            x: r.x.into(),
            y: r.y.into(),
            width: Some(r.width.into()),
            height: Some(r.height.into()),
            fill: ColorOrGradient::Color([0.0; 4]).into(),
            ..Default::default()
        }
        .into(),
    );
    group.marks.push(hit.into());
    group
}

impl WidgetRuntime {
    fn edit_text(
        &mut self,
        id: &WidgetId,
        action: Action,
        class: Option<EditClass>,
        update: &mut WidgetUpdate,
    ) -> Result<(), WidgetError> {
        let engine = self
            .engine
            .as_ref()
            .ok_or_else(|| {
                WidgetError::Invalid("text input must be prepared before editing".into())
            })?
            .clone();
        let c = self.controls.get_mut(id).unwrap();
        let WidgetSpec::TextInput(spec) = &mut c.spec else {
            return Ok(());
        };
        let s = c.text.as_mut().unwrap();
        let preedit = matches!(&action, Action::Preedit { .. });
        let ime_commit = matches!(&action, Action::Commit(_));
        let before = if ime_commit {
            s.composition
                .take()
                .unwrap_or_else(|| Snapshot::new(&s.editor))
        } else {
            Snapshot::new(&s.editor)
        };
        let typography = spec
            .text_style
            .as_ref()
            .unwrap_or(&self.theme.text_input.text);
        let changed_layout = s.editor.apply(action, &engine, &typography.config(""))?;
        update.status.rerender |= changed_layout;
        if !preedit {
            if let Some(class) = class {
                s.history
                    .push(before, Snapshot::new(&s.editor), class, self.now);
            } else {
                s.history.boundary();
            }
            changed(s, spec, c.epoch, self.namespace, self.now, update);
        }
        self.reset_text_blink(id, update);
        self.layout_text(update)?;
        Ok(())
    }
    fn finish_composition(
        &mut self,
        id: &WidgetId,
        text: String,
        update: &mut WidgetUpdate,
    ) -> Result<(), WidgetError> {
        let c = self.controls.get_mut(id).unwrap();
        let s = c.text.as_mut().unwrap();
        if s.deferred.is_some() {
            return self.cancel_composing_text(id, update);
        }
        let class = if s.composition.is_some() {
            EditClass::Separate
        } else {
            EditClass::Typing
        };
        self.edit_text(id, Action::Commit(text), Some(class), update)?;
        self.resume_text_commit(id, update);
        Ok(())
    }
    fn escape_text(&mut self, id: &WidgetId, update: &mut WidgetUpdate) -> Result<(), WidgetError> {
        let c = self.controls.get_mut(id).unwrap();
        let s = c.text.as_mut().unwrap();
        let WidgetSpec::TextInput(spec) = &mut c.spec else {
            unreachable!()
        };
        let composition = s.composition.is_some();
        if composition {
            discard_composition(s);
            if let Some(value) = s.deferred.take() {
                s.editor.replace_committed_text(&value);
                s.baseline = value.clone();
                spec.value = value;
                s.history = History::default();
                cancel_pending(s, &wake_key(self.namespace, c.epoch, "commit"), update);
            }
        } else {
            cancel_pending(s, &wake_key(self.namespace, c.epoch, "commit"), update);
            s.editor.replace_committed_text(&s.baseline);
            s.history = History::default();
            if spec.value != s.baseline {
                spec.value = s.baseline.clone();
                update.emit(
                    id,
                    WidgetAction::TextChanged {
                        value: spec.value.clone(),
                    },
                );
            }
        }
        update.emit(
            id,
            WidgetAction::TextCancelled {
                value: spec.value.clone(),
                reason: if composition {
                    TextCancelReason::Composition
                } else {
                    TextCancelReason::Escape
                },
            },
        );
        self.stop_text_drag(&WidgetTarget::new(id.clone()), update);
        if composition {
            update
                .status
                .commands
                .push(Command::SetImeAllowed { allowed: false });
            self.activate_text_session(&WidgetTarget::new(id.clone()), update);
            self.resume_text_commit(id, update);
        }
        self.reset_text_blink(id, update);
        self.layout_text(update)?;
        Ok(())
    }
    fn submit_text(&mut self, id: &WidgetId, update: &mut WidgetUpdate) {
        let c = self.controls.get_mut(id).unwrap();
        let s = c.text.as_mut().unwrap();
        let WidgetSpec::TextInput(spec) = &c.spec else {
            unreachable!()
        };
        update.status.commands.extend(
            s.debounce
                .flush(&wake_key(self.namespace, c.epoch, "commit"))
                .commands,
        );
        if s.pending {
            commit(s, id, spec.value.clone(), TextCommitReason::Enter, update);
        }
        s.history.boundary();
        update.emit(
            id,
            WidgetAction::TextSubmitted {
                value: spec.value.clone(),
            },
        );
    }
    fn history_text(
        &mut self,
        id: &WidgetId,
        redo: bool,
        update: &mut WidgetUpdate,
    ) -> Result<(), WidgetError> {
        let c = self.controls.get_mut(id).unwrap();
        let s = c.text.as_mut().unwrap();
        let WidgetSpec::TextInput(spec) = &mut c.spec else {
            unreachable!()
        };
        if let Some(snapshot) = s.history.restore(redo) {
            snapshot.restore(&mut s.editor);
            changed(s, spec, c.epoch, self.namespace, self.now, update);
            update.status.rerender = true;
        }
        self.reset_text_blink(id, update);
        self.layout_text(update)?;
        Ok(())
    }
    fn text_key(
        &mut self,
        id: &WidgetId,
        input: &avenger_eventstream::window::WindowKeyboardInput,
        m: ModifiersState,
        update: &mut WidgetUpdate,
    ) -> Result<(), WidgetError> {
        let key = input.key;
        let text = input.text.as_deref();
        let pressed = input.state == ElementState::Pressed;
        let repeat = input.repeat;
        let c = &self.controls[id];
        let WidgetSpec::TextInput(spec) = &c.spec else {
            unreachable!()
        };
        let composing = c.text.as_ref().unwrap().composition.is_some();
        let read_only = spec.read_only;
        if key == Key::Named(NamedKey::Escape) {
            update.status.consume = true;
            if pressed && !repeat {
                if self.gesture.as_ref().is_some_and(|g| {
                    g.target.widget == *id && matches!(g.kind, GestureKind::TextSelection)
                }) {
                    self.stop_text_drag(&WidgetTarget::new(id.clone()), update);
                } else {
                    self.escape_text(id, update)?;
                }
            }
            return Ok(());
        }
        if composing {
            update.status.consume = true;
            return Ok(());
        }
        let command = if self.shortcuts == TextShortcuts::Mac {
            m.meta
        } else {
            m.control
        };
        let word = if self.shortcuts == TextShortcuts::Mac {
            m.alt
        } else {
            m.control
        };
        if key == Key::Named(NamedKey::Enter) {
            update.status.consume = true;
            if pressed && !repeat {
                self.submit_text(id, update);
            }
            return Ok(());
        }
        if !pressed {
            return Ok(());
        }
        if read_only && matches!(key, Key::Named(NamedKey::Backspace | NamedKey::Delete)) {
            update.status.consume = true;
            return Ok(());
        }
        let action = match key {
            Key::Character('z' | 'Z') if command => {
                update.status.consume = true;
                if !read_only {
                    self.history_text(id, m.shift, update)?;
                }
                return Ok(());
            }
            Key::Character('y' | 'Y') if command => {
                update.status.consume = true;
                if !read_only {
                    self.history_text(id, true, update)?;
                }
                return Ok(());
            }
            Key::Character('a' | 'A') if command => Some((Action::SelectAll, None)),
            Key::Named(NamedKey::ArrowLeft) => Some((
                Action::Motion {
                    motion: if command && self.shortcuts == TextShortcuts::Mac {
                        Motion::Start
                    } else if word {
                        Motion::WordLeft
                    } else {
                        Motion::Left
                    },
                    extend: m.shift,
                },
                None,
            )),
            Key::Named(NamedKey::ArrowRight) => Some((
                Action::Motion {
                    motion: if command && self.shortcuts == TextShortcuts::Mac {
                        Motion::End
                    } else if word {
                        Motion::WordRight
                    } else {
                        Motion::Right
                    },
                    extend: m.shift,
                },
                None,
            )),
            Key::Named(NamedKey::Home) => Some((
                Action::Motion {
                    motion: Motion::Start,
                    extend: m.shift,
                },
                None,
            )),
            Key::Named(NamedKey::End) => Some((
                Action::Motion {
                    motion: Motion::End,
                    extend: m.shift,
                },
                None,
            )),
            Key::Named(NamedKey::Backspace) if !read_only => Some((
                if command && self.shortcuts == TextShortcuts::Mac {
                    Action::DeleteToStart
                } else if word {
                    Action::DeleteWordBack
                } else {
                    Action::Backspace
                },
                Some(EditClass::Backward),
            )),
            Key::Named(NamedKey::Delete) if !read_only => Some((
                if command && self.shortcuts == TextShortcuts::Mac {
                    Action::DeleteToEnd
                } else if word {
                    Action::DeleteWordForward
                } else {
                    Action::Delete
                },
                Some(EditClass::Forward),
            )),
            _ if !read_only && !command && !m.control && !m.meta => {
                text.map(|t| (Action::InsertText(t.into()), Some(EditClass::Typing)))
            }
            _ => None,
        };
        if let Some((action, class)) = action {
            update.status.consume = true;
            self.edit_text(id, action, class, update)?;
        }
        Ok(())
    }
    pub(crate) fn handle_text_event(
        &mut self,
        event: &Event,
        update: &mut WidgetUpdate,
    ) -> Result<bool, WidgetError> {
        if let Event::RuntimeWake(wake) = event {
            if wake.key.namespace != format!("avenger-widgets-{}", self.namespace) {
                return Ok(false);
            }
            let Some(id) = self
                .controls
                .iter()
                .find(|(_, c)| c.epoch == wake.key.attachment_epoch && c.text.is_some())
                .map(|(id, _)| id.clone())
            else {
                return Ok(true);
            };
            let c = self.controls.get_mut(&id).unwrap();
            let s = c.text.as_mut().unwrap();
            match wake.key.purpose.as_str() {
                "commit" => {
                    let result = s.debounce.handle_wakeup(wake, self.now);
                    update.status.commands.extend(result.commands);
                    if let Some(value) = result.commit
                        && s.composition.is_none()
                    {
                        commit(s, &id, value, TextCommitReason::Debounced, update);
                    }
                }
                "blink"
                    if s.session.is_some()
                        && wake.generation == s.blink_generation
                        && matches!(&c.spec,WidgetSpec::TextInput(t) if !t.read_only&&t.options.enabled) =>
                {
                    s.caret_visible = !s.caret_visible;
                    s.blink_generation += 1;
                    update.status.rerender = true;
                    update.status.commands.push(Command::RequestWakeup {
                        key: wake.key.clone(),
                        deadline: self.now + Duration::from_millis(500),
                        generation: s.blink_generation,
                    });
                }
                "scroll" if s.session.is_some() && wake.generation == s.scroll_generation => {
                    if let Some(x) = s.drag_x {
                        self.text_drag(x, update)?;
                    }
                }
                _ => {}
            }
            return Ok(true);
        }
        let Event::TextInput { input, modifiers } = event else {
            return Ok(false);
        };
        if self.active_input_session() != Some(&input.session) {
            return Ok(true);
        }
        let Some(target) = self.focused.clone() else {
            return Ok(true);
        };
        let id = target.widget;
        let WidgetSpec::TextInput(spec) = &self.controls[&id].spec else {
            return Ok(true);
        };
        let read_only = spec.read_only;
        let composing = self.controls[&id]
            .text
            .as_ref()
            .unwrap()
            .composition
            .is_some();
        match &input.event {
            TextInputEvent::Keyboard(k) => {
                if k.key == Key::Named(NamedKey::Tab) && !composing {
                    return Ok(false);
                }
                if k.state == ElementState::Pressed && k.key != Key::Named(NamedKey::Escape) {
                    self.stop_text_drag(&WidgetTarget::new(id.clone()), update);
                }
                self.text_key(&id, k, *modifiers, update)?;
            }
            TextInputEvent::Ime(ime) if !read_only => {
                update.status.consume = true;
                match ime {
                    ImeEvent::Preedit { text, cursor } => {
                        if normalize_single_line(text).is_empty() && composing {
                            self.cancel_composing_text(&id, update)?;
                        } else if !normalize_single_line(text).is_empty() {
                            self.stop_text_drag(&WidgetTarget::new(id.clone()), update);
                            let c = self.controls.get_mut(&id).unwrap();
                            let s = c.text.as_mut().unwrap();
                            if s.composition.is_none() {
                                s.composition = Some(Snapshot::new(&s.editor));
                                s.history.boundary();
                                update.status.commands.extend(
                                    s.debounce
                                        .cancel(&wake_key(self.namespace, c.epoch, "commit"))
                                        .commands,
                                );
                            }
                            self.edit_text(
                                &id,
                                Action::Preedit {
                                    text: text.to_string(),
                                    cursor: *cursor,
                                },
                                None,
                                update,
                            )?;
                        }
                    }
                    ImeEvent::Commit(text) => {
                        self.finish_composition(&id, text.to_string(), update)?
                    }
                    ImeEvent::Disabled => self.cancel_composing_text(&id, update)?,
                    ImeEvent::Enabled => {}
                }
            }
            TextInputEvent::Clipboard(event) if !composing => {
                update.status.consume = true;
                match event {
                    ClipboardEvent::Copy | ClipboardEvent::Cut => {
                        let selected = self.controls[&id]
                            .text
                            .as_ref()
                            .unwrap()
                            .editor
                            .selected_text()
                            .to_string();
                        if !selected.is_empty() {
                            update
                                .status
                                .commands
                                .push(Command::WriteClipboard { text: selected });
                            if matches!(event, ClipboardEvent::Cut) && !read_only {
                                self.edit_text(
                                    &id,
                                    Action::Backspace,
                                    Some(EditClass::Separate),
                                    update,
                                )?;
                            }
                        }
                        self.controls
                            .get_mut(&id)
                            .unwrap()
                            .text
                            .as_mut()
                            .unwrap()
                            .history
                            .boundary();
                    }
                    ClipboardEvent::Paste(text) if !read_only => self.edit_text(
                        &id,
                        Action::InsertText(text.to_string()),
                        Some(EditClass::Separate),
                        update,
                    )?,
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(true)
    }
    pub(crate) fn text_press(
        &mut self,
        target: &WidgetTarget,
        position: [f32; 2],
        update: &mut WidgetUpdate,
    ) -> Result<(), WidgetError> {
        let Some(s) = self
            .controls
            .get_mut(&target.widget)
            .and_then(|c| c.text.as_mut())
        else {
            return Ok(());
        };
        if s.composition.is_some() {
            return Ok(());
        }
        let Some(l) = &s.layout else {
            return Ok(());
        };
        let x = position[0] - l.origin[0];
        let count = s
            .last_click
            .filter(|(time, p, _)| {
                self.now.saturating_duration_since(*time) <= Duration::from_millis(500)
                    && (position[0] - p[0]).abs() <= 4.0
                    && (position[1] - p[1]).abs() <= 4.0
            })
            .map_or(1, |(_, _, count)| count % 3 + 1);
        s.last_click = Some((self.now, position, count));
        s.drag_x = Some(position[0]);
        if let Some(g) = &mut self.gesture {
            g.kind = GestureKind::TextSelection;
        }
        let action = match count {
            1 => Action::Click { x },
            2 => Action::DoubleClick { x },
            _ => Action::TripleClick,
        };
        self.edit_text(&target.widget, action, None, update)
    }
    pub(crate) fn text_drag(
        &mut self,
        x: f32,
        update: &mut WidgetUpdate,
    ) -> Result<(), WidgetError> {
        let Some(g) = &self.gesture else {
            return Ok(());
        };
        if !matches!(g.kind, GestureKind::TextSelection) {
            return Ok(());
        }
        let id = g.target.widget.clone();
        let c = self.controls.get_mut(&id).unwrap();
        let s = c.text.as_mut().unwrap();
        let Some(l) = &s.layout else {
            return Ok(());
        };
        s.drag_x = Some(x);
        let edge = if x < l.inner.x {
            x - l.inner.x
        } else if x > l.inner.x + l.inner.width {
            x - l.inner.x - l.inner.width
        } else {
            0.0
        };
        if edge != 0.0 {
            s.scroll = (s.scroll + edge.clamp(-24.0, 24.0))
                .clamp(0.0, (l.line.bounds.width + 1.0 - l.inner.width).max(0.0));
        }
        let local_x = x - l.inner.x + s.scroll;
        s.scroll_generation += 1;
        let key = wake_key(self.namespace, c.epoch, "scroll");
        if edge != 0.0 {
            update.status.commands.push(Command::RequestWakeup {
                key,
                deadline: self.now + Duration::from_millis(16),
                generation: s.scroll_generation,
            });
        } else {
            update.status.commands.push(Command::CancelWakeup { key });
        }
        self.edit_text(&id, Action::Drag { x: local_x }, None, update)
    }
    pub(crate) fn stop_text_drag(&mut self, target: &WidgetTarget, update: &mut WidgetUpdate) {
        if let Some(c) = self.controls.get_mut(&target.widget)
            && let Some(s) = &mut c.text
        {
            s.drag_x = None;
            s.scroll_generation += 1;
            update.status.commands.push(Command::CancelWakeup {
                key: wake_key(self.namespace, c.epoch, "scroll"),
            });
        }
        if self
            .gesture
            .as_ref()
            .is_some_and(|g| &g.target == target && matches!(g.kind, GestureKind::TextSelection))
        {
            self.gesture = None;
            update
                .status
                .commands
                .push(Command::SetPointerCapture { captured: false });
        }
    }
}

impl WidgetRuntime {
    fn resume_text_commit(&mut self, id: &WidgetId, update: &mut WidgetUpdate) {
        let c = self.controls.get_mut(id).unwrap();
        let s = c.text.as_mut().unwrap();
        let WidgetSpec::TextInput(spec) = &c.spec else {
            return;
        };
        if s.pending
            && s.composition.is_none()
            && matches!(spec.policy, TextCommitPolicy::Debounced(_))
            && s.debounce.pending_generation().is_none()
        {
            let result = s.debounce.submit(
                spec.value.clone(),
                self.now,
                &wake_key(self.namespace, c.epoch, "commit"),
            );
            update.status.commands.extend(result.commands);
            if let Some(value) = result.commit {
                commit(s, id, value, TextCommitReason::Immediate, update);
            }
        }
    }
}

impl WidgetRuntime {
    fn cancel_composing_text(
        &mut self,
        id: &WidgetId,
        update: &mut WidgetUpdate,
    ) -> Result<(), WidgetError> {
        let c = self.controls.get_mut(id).unwrap();
        let s = c.text.as_mut().unwrap();
        discard_composition(s);
        let replaced = if let Some(value) = s.deferred.take() {
            s.editor.replace_committed_text(&value);
            s.baseline = value.clone();
            s.history = History::default();
            cancel_pending(s, &wake_key(self.namespace, c.epoch, "commit"), update);
            if let WidgetSpec::TextInput(spec) = &mut c.spec {
                spec.value = value;
            }
            true
        } else {
            false
        };
        if replaced {
            self.activate_text_session(&WidgetTarget::new(id.clone()), update);
        }
        self.resume_text_commit(id, update);
        self.reset_text_blink(id, update);
        update.status.rerender = true;
        self.layout_text(update)
    }
}
