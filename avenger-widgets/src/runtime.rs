use crate::frame::{Region, visible};
use crate::{
    ChoiceItemId, Rect, SliderCancelReason, SliderDomain, WidgetError, WidgetId, WidgetSpec,
    WidgetTarget, WidgetTheme,
};
use avenger_common::{cursor::CursorStyle, time::Instant};
use avenger_eventstream::{
    runtime::{KeyboardPolicy, RuntimeHostCommand as Command},
    scene::{ModifiersState, SceneGraphEvent as Event},
    stream::UpdateStatus,
    window::{ElementState, Key, MouseButton, NamedKey, TextInputEvent},
};
use avenger_geometry::rtree::SceneGraphRTree;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicU64, Ordering},
};

/// What Tab does at the edges of the widget region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FocusBoundary {
    /// Continue into surrounding host controls.
    #[default]
    Handoff,
    /// Wrap within a standalone application window.
    Cycle,
}
/// A user intention. Applications apply proposed values before the next frame.
#[derive(Clone, Debug, PartialEq)]
pub enum WidgetAction {
    TextChanged {
        value: String,
    },
    TextCommitted {
        value: String,
        reason: crate::TextCommitReason,
    },
    TextSubmitted {
        value: String,
    },
    TextCancelled {
        value: String,
        reason: crate::TextCancelReason,
    },
    Activated,
    CheckedItemsChanged {
        item: ChoiceItemId,
        checked: BTreeSet<ChoiceItemId>,
    },
    SelectionChanged {
        item: ChoiceItemId,
    },
    SliderChanged {
        value: f64,
    },
    SliderCommitted {
        value: f64,
    },
    SliderCancelled {
        value: f64,
        reason: SliderCancelReason,
    },
    CheckedChanged {
        value: bool,
    },
    FocusChanged {
        focused: bool,
        item: Option<crate::ChoiceItemId>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetEvent {
    pub id: WidgetId,
    pub action: WidgetAction,
}
/// Application actions plus the existing eventstream invalidation/host protocol.
#[derive(Clone, Debug, Default)]
pub struct WidgetUpdate {
    pub events: Vec<WidgetEvent>,
    pub status: UpdateStatus,
}
impl WidgetUpdate {
    pub(crate) fn emit(&mut self, id: &WidgetId, action: WidgetAction) {
        self.events.push(WidgetEvent {
            id: id.clone(),
            action,
        });
        self.status.rerender = true;
    }
}
/// Host-neutral role metadata; this is not a platform accessibility bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetRole {
    Button,
    Checkbox,
    CheckboxGroup,
    RadioGroup,
    Radio,
    Slider,
    TextInput,
}
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticValue {
    None,
    Checked(bool),
    CheckedItems(BTreeSet<ChoiceItemId>),
    Selected(Option<ChoiceItemId>),
    Number(f64),
    Text(String),
}
/// Installed identity, naming, state and geometry for external adapters.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetSemantic {
    pub target: WidgetTarget,
    pub role: WidgetRole,
    pub name: String,
    pub enabled: bool,
    pub focused: bool,
    pub value: SemanticValue,
    pub bounds: Rect,
    pub parent: Option<WidgetTarget>,
    pub domain: Option<SliderDomain>,
    pub read_only: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Control {
    pub spec: WidgetSpec,
    pub epoch: u64,
    pub item_epochs: BTreeMap<ChoiceItemId, u64>,
    pub text: Option<Box<crate::text_input::TextState>>,
}
#[derive(Clone, Debug)]
pub(crate) struct Gesture {
    pub target: WidgetTarget,
    pub epoch: u64,
    pub keyboard: bool,
    pub inside: bool,
    pub kind: GestureKind,
}

#[derive(Clone, Debug)]
pub(crate) enum GestureKind {
    Button,
    TextSelection,
    Slider {
        start: f64,
        last: f64,
        changed: bool,
        key: Option<Key>,
        grab: f32,
    },
}

/// Persistent focus and interaction state. Clones are isolated scene-build candidates.
#[derive(Clone, Debug)]
pub struct WidgetRuntime {
    pub(crate) namespace: u64,
    pub(crate) revision: u64,
    pub(crate) next_epoch: u64,
    pub(crate) controls: BTreeMap<WidgetId, Control>,
    pub(crate) order: Vec<WidgetId>,
    pub(crate) regions: Vec<Region>,
    pub(crate) theme: WidgetTheme,
    pub(crate) focused: Option<WidgetTarget>,
    pub(crate) focus_visible: bool,
    pub(crate) hovered: Option<WidgetTarget>,
    pub(crate) gesture: Option<Gesture>,
    remembered: Option<WidgetTarget>,
    boundary: FocusBoundary,
    pub(crate) engine: Option<avenger_text::TextEngine>,
    pub(crate) now: Instant,
    pub(crate) next_session: u64,
    pub(crate) shortcuts: crate::TextShortcuts,
}
static NEXT_RUNTIME: AtomicU64 = AtomicU64::new(1);
impl Default for WidgetRuntime {
    fn default() -> Self {
        Self {
            namespace: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
            revision: 0,
            next_epoch: 0,
            controls: BTreeMap::new(),
            order: Vec::new(),
            regions: Vec::new(),
            theme: WidgetTheme::default(),
            focused: None,
            focus_visible: false,
            hovered: None,
            gesture: None,
            remembered: None,
            boundary: FocusBoundary::Handoff,
            engine: None,
            now: Instant::now(),
            next_session: 0,
            shortcuts: if cfg!(target_os = "macos") {
                crate::TextShortcuts::Mac
            } else {
                crate::TextShortcuts::Control
            },
        }
    }
}
impl WidgetRuntime {
    pub fn new() -> Self {
        Self::default()
    }
    /// Choose native cycling or browser boundary handoff before preparing a frame.
    pub fn with_focus_boundary(mut self, boundary: FocusBoundary) -> Self {
        self.boundary = boundary;
        self
    }
    pub fn with_text_shortcuts(mut self, shortcuts: crate::TextShortcuts) -> Self {
        self.shortcuts = shortcuts;
        self
    }
    pub fn focused(&self) -> Option<&WidgetTarget> {
        self.focused.as_ref()
    }
    /// Request visible logical focus, or blur. The caller delivers returned host effects.
    pub fn request_focus(
        &mut self,
        target: Option<WidgetTarget>,
        now: Instant,
    ) -> Result<WidgetUpdate, WidgetError> {
        self.now = now;
        if target.as_ref().is_some_and(|t| !self.eligible(t)) {
            return Err(WidgetError::Invalid(
                "focus target must be enabled, placed, and visible".into(),
            ));
        }
        let mut update = WidgetUpdate::default();
        self.focus(target, true, &mut update);
        self.revision += 1;
        self.publish_policy(&mut update);
        Ok(update)
    }
    /// Snapshot semantic metadata from the installed frame.
    pub fn semantics(&self) -> Vec<WidgetSemantic> {
        self.regions
            .iter()
            .map(|r| {
                let c = &self.controls[&r.target.widget];
                let item = r
                    .target
                    .item
                    .as_ref()
                    .and_then(|id| c.spec.items()?.iter().find(|i| &i.id == id));
                let (role, value, domain) = match &c.spec {
                    WidgetSpec::Button(_) => (WidgetRole::Button, SemanticValue::None, None),
                    WidgetSpec::Checkbox(c) => (
                        WidgetRole::Checkbox,
                        SemanticValue::Checked(c.checked),
                        None,
                    ),
                    WidgetSpec::CheckboxGroup(g) => {
                        if let Some(item) = item {
                            (
                                WidgetRole::Checkbox,
                                SemanticValue::Checked(g.checked.contains(&item.id)),
                                None,
                            )
                        } else {
                            (
                                WidgetRole::CheckboxGroup,
                                SemanticValue::CheckedItems(g.checked.clone()),
                                None,
                            )
                        }
                    }
                    WidgetSpec::RadioGroup(g) => {
                        if let Some(item) = item {
                            (
                                WidgetRole::Radio,
                                SemanticValue::Checked(g.selected.as_ref() == Some(&item.id)),
                                None,
                            )
                        } else {
                            (
                                WidgetRole::RadioGroup,
                                SemanticValue::Selected(g.selected.clone()),
                                None,
                            )
                        }
                    }
                    WidgetSpec::TextInput(t) => (
                        WidgetRole::TextInput,
                        SemanticValue::Text(t.value.clone()),
                        None,
                    ),
                    WidgetSpec::Slider(s) => (
                        WidgetRole::Slider,
                        SemanticValue::Number(s.value),
                        Some(s.domain),
                    ),
                };
                WidgetSemantic {
                    target: r.target.clone(),
                    role,
                    value,
                    domain,
                    read_only: matches!(&c.spec,WidgetSpec::TextInput(t) if t.read_only),
                    parent: item.map(|_| WidgetTarget::new(r.target.widget.clone())),
                    name: item.map(|i| i.label.clone()).unwrap_or_else(|| {
                        c.spec
                            .options()
                            .semantic_name
                            .clone()
                            .unwrap_or_else(|| c.spec.label().into())
                    }),
                    enabled: c.spec.options().enabled && item.is_none_or(|i| i.enabled),
                    focused: self.focused.as_ref() == Some(&r.target),
                    bounds: r.clip,
                }
            })
            .collect()
    }
    pub(crate) fn target_epoch(&self, t: &WidgetTarget) -> Option<u64> {
        let c = self.controls.get(&t.widget)?;
        if !c.spec.options().enabled {
            return None;
        }
        if let Some(item) = &t.item {
            c.item_epochs.get(item).copied()
        } else if c.spec.items().is_none() {
            Some(c.epoch)
        } else {
            None
        }
    }
    pub(crate) fn eligible(&self, t: &WidgetTarget) -> bool {
        self.target_epoch(t).is_some()
            && self
                .regions
                .iter()
                .any(|r| &r.target == t && visible(r.clip))
    }
    pub(crate) fn stops(&self) -> Vec<WidgetTarget> {
        let mut stops = Vec::new();
        for id in &self.order {
            let c = &self.controls[id];
            let targets: Vec<_> = self
                .regions
                .iter()
                .map(|r| r.target.clone())
                .filter(|t| &t.widget == id && self.eligible(t))
                .collect();
            if let WidgetSpec::RadioGroup(g) = &c.spec {
                let entry = self
                    .focused
                    .as_ref()
                    .filter(|t| targets.contains(t))
                    .cloned()
                    .or_else(|| {
                        g.selected.as_ref().and_then(|item| {
                            targets
                                .iter()
                                .find(|t| t.item.as_ref() == Some(item))
                                .cloned()
                        })
                    })
                    .or_else(|| targets.first().cloned());
                stops.extend(entry);
            } else {
                stops.extend(targets);
            }
        }
        stops
    }
    pub(crate) fn pressed(&self, t: &WidgetTarget) -> bool {
        self.gesture
            .as_ref()
            .is_some_and(|g| &g.target == t && g.inside)
    }
    pub(crate) fn cancel_gesture(&mut self, reason: SliderCancelReason, update: &mut WidgetUpdate) {
        if let Some(g) = self.gesture.take() {
            if matches!(g.kind, GestureKind::TextSelection) {
                self.stop_text_drag(&g.target, update);
            }
            if let GestureKind::Slider { start, last, .. } = g.kind {
                let mut value = if let Some(Control {
                    spec: WidgetSpec::Slider(s),
                    ..
                }) = self.controls.get(&g.target.widget)
                {
                    s.value
                } else {
                    last
                };
                if reason == SliderCancelReason::Escape {
                    value = start;
                    self.set_slider(&g.target, value, update);
                }
                update.emit(
                    &g.target.widget,
                    WidgetAction::SliderCancelled { value, reason },
                );
            }
            update.status.rerender = true;
            if !g.keyboard {
                update
                    .status
                    .commands
                    .push(Command::SetPointerCapture { captured: false });
            }
        }
    }
    pub(crate) fn reconcile_targets(&mut self, update: &mut WidgetUpdate) {
        if self.gesture.as_ref().is_some_and(|g| {
            !self.eligible(&g.target) || self.target_epoch(&g.target) != Some(g.epoch)
        }) {
            let reason = if self
                .gesture
                .as_ref()
                .is_some_and(|g| self.controls.contains_key(&g.target.widget))
            {
                SliderCancelReason::Disabled
            } else {
                SliderCancelReason::Removed
            };
            self.cancel_gesture(reason, update);
        }
        if self.focused.as_ref().is_some_and(|t| !self.eligible(t)) {
            if let Some(t) = self.focused.clone() {
                self.deactivate_text(
                    &t,
                    Some(if self.controls.contains_key(&t.widget) {
                        crate::TextCancelReason::Disabled
                    } else {
                        crate::TextCancelReason::Removed
                    }),
                    update,
                );
            }
            self.focus(None, false, update);
        }
        if self.remembered.as_ref().is_some_and(|t| !self.eligible(t)) {
            self.remembered = None;
        }
        if self.hovered.as_ref().is_some_and(|t| !self.eligible(t)) {
            self.hovered = None;
        }
    }
    pub(crate) fn focus(
        &mut self,
        target: Option<WidgetTarget>,
        visible: bool,
        update: &mut WidgetUpdate,
    ) {
        if self.focused == target {
            if self.focus_visible != visible {
                self.focus_visible = visible;
                update.status.rerender = true;
            }
            return;
        }
        self.cancel_gesture(SliderCancelReason::FocusLost, update);
        if let Some(old) = self.focused.take() {
            self.deactivate_text(&old, None, update);
            update.emit(
                &old.widget,
                WidgetAction::FocusChanged {
                    focused: false,
                    item: old.item,
                },
            );
        }
        self.focused = target;
        self.focus_visible = visible;
        self.remembered = None;
        if let Some(new) = self.focused.clone() {
            self.activate_text_session(&new, update);
        }
        if let Some(new) = &self.focused {
            update.emit(
                &new.widget,
                WidgetAction::FocusChanged {
                    focused: true,
                    item: new.item.clone(),
                },
            );
        }
    }
    pub(crate) fn publish_policy(&self, update: &mut WidgetUpdate) {
        let stops = self.stops();
        let index = self
            .focused
            .as_ref()
            .and_then(|t| stops.iter().position(|s| s == t));
        let mut policy = KeyboardPolicy {
            tab_forward: !stops.is_empty()
                && (self.boundary == FocusBoundary::Cycle
                    || index.is_none_or(|i| i + 1 < stops.len())),
            tab_backward: !stops.is_empty()
                && (self.boundary == FocusBoundary::Cycle || index.is_none_or(|i| i > 0)),
            ..Default::default()
        };
        if let Some(t) = &self.focused {
            policy.keys = match &self.controls[&t.widget].spec {
                WidgetSpec::Button(_) => vec![NamedKey::Enter, NamedKey::Space, NamedKey::Escape],
                WidgetSpec::Checkbox(_) | WidgetSpec::CheckboxGroup(_) => {
                    vec![NamedKey::Space, NamedKey::Escape]
                }
                WidgetSpec::RadioGroup(_) => vec![
                    NamedKey::Space,
                    NamedKey::Escape,
                    NamedKey::ArrowLeft,
                    NamedKey::ArrowRight,
                    NamedKey::ArrowUp,
                    NamedKey::ArrowDown,
                ],
                WidgetSpec::TextInput(_) => vec![
                    NamedKey::Backspace,
                    NamedKey::Delete,
                    NamedKey::ArrowLeft,
                    NamedKey::ArrowRight,
                    NamedKey::Home,
                    NamedKey::End,
                    NamedKey::Enter,
                    NamedKey::Escape,
                ],
                WidgetSpec::Slider(_) => vec![
                    NamedKey::Escape,
                    NamedKey::ArrowLeft,
                    NamedKey::ArrowRight,
                    NamedKey::ArrowUp,
                    NamedKey::ArrowDown,
                    NamedKey::Home,
                    NamedKey::End,
                    NamedKey::PageUp,
                    NamedKey::PageDown,
                ],
            };
        }
        policy.text_shortcuts = self
            .focused
            .as_ref()
            .is_some_and(|t| matches!(self.controls[&t.widget].spec, WidgetSpec::TextInput(_)));
        update.status.commands.push(Command::SetKeyboardPolicy {
            policy: Some(policy),
        });
        self.publish_text_host(update);
    }
    fn activate(&mut self, target: &WidgetTarget, update: &mut WidgetUpdate) {
        if !self.eligible(target) {
            return;
        }
        match &mut self.controls.get_mut(&target.widget).unwrap().spec {
            WidgetSpec::Button(_) => update.emit(&target.widget, WidgetAction::Activated),
            WidgetSpec::CheckboxGroup(g) => {
                if let Some(item) = &target.item {
                    if !g.checked.remove(item) {
                        g.checked.insert(item.clone());
                    }
                    update.emit(
                        &target.widget,
                        WidgetAction::CheckedItemsChanged {
                            item: item.clone(),
                            checked: g.checked.clone(),
                        },
                    );
                }
            }
            WidgetSpec::RadioGroup(g) => {
                if let Some(item) = &target.item
                    && g.selected.as_ref() != Some(item)
                {
                    g.selected = Some(item.clone());
                    update.emit(
                        &target.widget,
                        WidgetAction::SelectionChanged { item: item.clone() },
                    );
                }
            }
            WidgetSpec::Slider(_) | WidgetSpec::TextInput(_) => {}
            WidgetSpec::Checkbox(c) => {
                c.checked = !c.checked;
                update.emit(
                    &target.widget,
                    WidgetAction::CheckedChanged { value: c.checked },
                );
            }
        }
    }
    fn key(
        &mut self,
        key: Key,
        pressed: bool,
        repeat: bool,
        modifiers: ModifiersState,
        update: &mut WidgetUpdate,
    ) {
        if key == Key::Named(NamedKey::Tab) && pressed {
            if repeat || modifiers.control || modifiers.alt || modifiers.meta {
                return;
            }
            let stops = self.stops();
            if stops.is_empty() {
                return;
            }
            let old = self
                .focused
                .as_ref()
                .and_then(|t| stops.iter().position(|s| s == t));
            let next = match old {
                None => Some(if modifiers.shift { stops.len() - 1 } else { 0 }),
                Some(i) if modifiers.shift => i
                    .checked_sub(1)
                    .or((self.boundary == FocusBoundary::Cycle).then_some(stops.len() - 1)),
                Some(i) => {
                    if i + 1 < stops.len() {
                        Some(i + 1)
                    } else {
                        (self.boundary == FocusBoundary::Cycle).then_some(0)
                    }
                }
            };
            self.focus(next.map(|i| stops[i].clone()), true, update);
            update.status.consume = next.is_some();
            return;
        }
        let Some(target) = self.focused.clone() else {
            return;
        };
        if matches!(self.controls[&target.widget].spec, WidgetSpec::TextInput(_)) {
            return;
        }
        if pressed && (modifiers.control || modifiers.alt || modifiers.meta) {
            return;
        }
        if key == Key::Named(NamedKey::Escape) && pressed && self.gesture.is_some() {
            self.cancel_gesture(SliderCancelReason::Escape, update);
            update.status.consume = true;
            return;
        }
        if matches!(self.controls[&target.widget].spec, WidgetSpec::Slider(_)) {
            self.slider_key(&target, key, pressed, update);
            return;
        }
        if pressed && self.radio_key(&target, key, update) {
            return;
        }
        match key {
            Key::Named(NamedKey::Enter)
                if matches!(self.controls[&target.widget].spec, WidgetSpec::Button(_)) =>
            {
                if pressed && !repeat {
                    self.activate(&target, update);
                }
                update.status.consume = true;
            }
            Key::Named(NamedKey::Space) | Key::Character(' ') => {
                update.status.consume = true;
                if pressed && !repeat && self.gesture.is_none() {
                    self.gesture = Some(Gesture {
                        epoch: self.target_epoch(&target).unwrap(),
                        target,
                        keyboard: true,
                        inside: true,
                        kind: GestureKind::Button,
                    });
                    update.status.rerender = true;
                } else if !pressed
                    && self
                        .gesture
                        .as_ref()
                        .is_some_and(|g| g.keyboard && g.target == target)
                {
                    self.gesture = None;
                    self.activate(&target, update);
                    update.status.rerender = true;
                }
            }
            _ => {}
        }
    }
    /// Route against the complete installed scene's picking order and clips.
    /// Apply output actions synchronously, then rebuild when `status.rerender` is set.
    pub fn handle(
        &mut self,
        event: &Event,
        rtree: &SceneGraphRTree,
        now: Instant,
    ) -> Result<WidgetUpdate, WidgetError> {
        self.now = now;
        let mut update = WidgetUpdate::default();
        let hit = event
            .position()
            .and_then(|p| rtree.pick_top_mark_at_point(&p))
            .and_then(|m| {
                self.regions
                    .iter()
                    .find(|r| !r.name.is_empty() && r.name == m.name)
            })
            .map(|r| r.target.clone());
        if self.handle_text_event(event, &mut update)? {
            self.revision += 1;
            self.publish_policy(&mut update);
            return Ok(update);
        }
        match event {
            Event::MouseDown(e) if e.button == MouseButton::Left => {
                if let Some(target) = hit {
                    update.status.consume = true;
                    update.status.suppress_click = true;
                    if self.eligible(&target) {
                        self.focus(Some(target.clone()), false, &mut update);
                        self.gesture = Some(Gesture {
                            epoch: self.target_epoch(&target).unwrap(),
                            target,
                            keyboard: false,
                            inside: true,
                            kind: GestureKind::Button,
                        });
                        if let Some(g) = &self.gesture {
                            let target = g.target.clone();
                            self.slider_press(&target, e.position[0], &mut update);
                            self.text_press(&target, e.position, &mut update)?;
                        }
                        update
                            .status
                            .commands
                            .push(Command::SetPointerCapture { captured: true });
                        update.status.rerender = true;
                    }
                } else {
                    self.focus(None, false, &mut update);
                }
            }
            Event::CursorMoved(_) => {
                if self.hovered != hit {
                    self.hovered = hit.clone();
                    update.status.rerender = true;
                    update.status.cursor = Some(hit.as_ref().filter(|t| self.eligible(t)).map_or(
                        CursorStyle::Default,
                        |t| match self.controls[&t.widget].spec {
                            WidgetSpec::TextInput(_) => CursorStyle::Text,
                            WidgetSpec::Slider(_) => CursorStyle::ResizeHorizontal,
                            _ => CursorStyle::Pointer,
                        },
                    ));
                }
                if let Some(g) = &mut self.gesture
                    && !g.keyboard
                {
                    let inside = matches!(g.kind, GestureKind::Slider { .. })
                        || hit.as_ref() == Some(&g.target);
                    update.status.rerender |= g.inside != inside;
                    g.inside = inside;
                    update.status.consume = true;
                }
                if let Event::CursorMoved(e) = event {
                    self.slider_move(e.position[0], &mut update);
                    self.text_drag(e.position[0], &mut update)?;
                }
            }
            Event::MouseUp(e) if e.button == MouseButton::Left => {
                if self.gesture.as_ref().is_some_and(|g| !g.keyboard) {
                    let g = self.gesture.take().unwrap();
                    update.status.consume = true;
                    update.status.suppress_click = true;
                    update.status.rerender = true;
                    update
                        .status
                        .commands
                        .push(Command::SetPointerCapture { captured: false });
                    if matches!(g.kind, GestureKind::TextSelection) {
                        self.stop_text_drag(&g.target, &mut update);
                    } else if matches!(g.kind, GestureKind::Slider { .. }) {
                        self.finish_slider(g, &mut update);
                    } else if hit.as_ref() == Some(&g.target) {
                        self.activate(&g.target, &mut update);
                    }
                }
            }
            Event::MouseLeave(e) => {
                if self.hovered.as_ref().is_some_and(|t| {
                    self.regions
                        .iter()
                        .any(|r| &r.target == t && r.name == e.mark_instance.name)
                }) {
                    self.hovered = None;
                    update.status.rerender = true;
                    update.status.cursor = Some(CursorStyle::Default);
                    if let Some(g) = &mut self.gesture
                        && matches!(g.kind, GestureKind::Button)
                    {
                        g.inside = false;
                    }
                }
            }
            Event::KeyPress(e) => self.key(e.key, true, e.repeat, e.modifiers, &mut update),
            Event::KeyRelease(e) => self.key(e.key, false, false, e.modifiers, &mut update),
            Event::TextInput { input, modifiers } => {
                if self.active_input_session() == Some(&input.session)
                    && let TextInputEvent::Keyboard(k) = &input.event
                {
                    self.key(
                        k.key,
                        k.state == ElementState::Pressed,
                        k.repeat,
                        *modifiers,
                        &mut update,
                    );
                }
            }
            Event::FocusEntered { reverse } => {
                let stops = self.stops();
                self.focus(
                    if *reverse {
                        stops.last()
                    } else {
                        stops.first()
                    }
                    .cloned(),
                    true,
                    &mut update,
                );
            }
            Event::WindowFocused(false) => {
                let remembered = self.focused.clone();
                self.focus(None, false, &mut update);
                self.remembered = remembered;
                self.hovered = None;
            }
            Event::WindowFocused(true) => {
                if let Some(t) = self.remembered.take().filter(|t| self.eligible(t)) {
                    self.focus(Some(t), true, &mut update);
                }
            }
            Event::PointerCaptureLost => {
                self.cancel_gesture(SliderCancelReason::CaptureLost, &mut update)
            }
            Event::WindowCloseRequested => {
                self.focus(None, false, &mut update);
                self.remembered = None;
            }
            _ => {}
        }
        self.revision += 1;
        self.publish_policy(&mut update);
        Ok(update)
    }
}

impl WidgetRuntime {
    pub(crate) fn reconcile_spec(
        &mut self,
        spec: &WidgetSpec,
        update: &mut WidgetUpdate,
    ) -> Result<(), WidgetError> {
        let mut spec = spec.clone();
        match &mut spec {
            WidgetSpec::CheckboxGroup(g) => {
                crate::choice::validate(&g.items, g.checked.iter().cloned())?
            }
            WidgetSpec::RadioGroup(g) => {
                crate::choice::validate(&g.items, g.selected.iter().cloned())?
            }
            WidgetSpec::Slider(s) => s.value = s.domain.normalize(s.value)?,
            WidgetSpec::TextInput(t) => {
                if let Some(style) = &t.text_style {
                    style.validate()?;
                }
                if let crate::TextCommitPolicy::Debounced(d) = t.policy
                    && (d.as_millis() >= u128::from(u64::MAX) || self.now.checked_add(d).is_none())
                {
                    return Err(WidgetError::Invalid(
                        "text debounce exceeds the host time range".into(),
                    ));
                }
            }
            _ => {}
        }
        let mut replaced_slider = false;
        if let Some(old) = self.controls.get(spec.id()) {
            if !old.spec.same_kind(&spec) {
                return Err(WidgetError::Invalid(format!(
                    "live widget {} changed kind",
                    spec.id().as_str()
                )));
            }
            if let (WidgetSpec::Slider(old), WidgetSpec::Slider(new)) = (&old.spec, &spec) {
                replaced_slider = old.value != new.value || old.domain != new.domain;
            }
        } else {
            self.next_epoch += 1;
            self.controls.insert(
                spec.id().clone(),
                Control {
                    spec: spec.clone(),
                    epoch: self.next_epoch,
                    item_epochs: BTreeMap::new(),
                    text: if let WidgetSpec::TextInput(t) = &spec {
                        Some(Box::new(crate::text_input::TextState::new(t)))
                    } else {
                        None
                    },
                },
            );
        }
        let id = spec.id().clone();
        self.reconcile_text(&spec, update);
        let c = self.controls.get_mut(&id).unwrap();
        if let Some(items) = spec.items() {
            c.item_epochs
                .retain(|id, _| items.iter().any(|i| &i.id == id && i.enabled));
            for item in items.iter().filter(|i| i.enabled) {
                if !c.item_epochs.contains_key(&item.id) {
                    self.next_epoch += 1;
                    c.item_epochs.insert(item.id.clone(), self.next_epoch);
                }
            }
        }
        c.spec = spec;
        if c.spec.options().enabled
            && c.text.as_ref().is_some_and(|s| s.session.is_none())
            && self.focused.as_ref().is_some_and(|t| t.widget == id)
        {
            self.activate_text_session(&WidgetTarget::new(id.clone()), update);
        }
        if replaced_slider && self.gesture.as_ref().is_some_and(|g| g.target.widget == id) {
            self.cancel_gesture(SliderCancelReason::Replaced, update);
        }
        Ok(())
    }
    fn radio_key(&mut self, target: &WidgetTarget, key: Key, update: &mut WidgetUpdate) -> bool {
        if !matches!(
            self.controls[&target.widget].spec,
            WidgetSpec::RadioGroup(_)
        ) {
            return false;
        }
        let forward = match key {
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown) => true,
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => false,
            _ => return false,
        };
        let targets: Vec<_> = self
            .regions
            .iter()
            .map(|r| r.target.clone())
            .filter(|t| t.widget == target.widget && self.eligible(t))
            .collect();
        if let Some(i) = targets.iter().position(|t| t == target) {
            let next = targets[if forward {
                (i + 1) % targets.len()
            } else {
                (i + targets.len() - 1) % targets.len()
            }]
            .clone();
            self.focus(Some(next.clone()), true, update);
            self.activate(&next, update);
        }
        update.status.consume = true;
        true
    }
    pub(crate) fn set_slider(
        &mut self,
        target: &WidgetTarget,
        value: f64,
        update: &mut WidgetUpdate,
    ) {
        if let Some(Control {
            spec: WidgetSpec::Slider(s),
            ..
        }) = self.controls.get_mut(&target.widget)
            && s.value != value
        {
            s.value = value;
            update.emit(&target.widget, WidgetAction::SliderChanged { value });
            if let Some(Gesture {
                kind: GestureKind::Slider { changed, last, .. },
                ..
            }) = &mut self.gesture
            {
                *changed = true;
                *last = value;
            }
        }
    }
    fn slider_press(&mut self, target: &WidgetTarget, x: f32, update: &mut WidgetUpdate) {
        let WidgetSpec::Slider(s) = &self.controls[&target.widget].spec else {
            return;
        };
        let region = self.regions.iter().find(|r| &r.target == target).unwrap();
        let (left, length) =
            crate::slider::track(region.rect, &self.theme.slider, s.value_label.is_some());
        let thumb = left + length * s.domain.fraction(s.value);
        let grab = if (x - thumb).abs() <= self.theme.slider.thumb_size / 2.0 {
            x - thumb
        } else {
            0.0
        };
        if let Some(g) = &mut self.gesture {
            g.kind = GestureKind::Slider {
                start: s.value,
                last: s.value,
                changed: false,
                key: None,
                grab,
            };
        }
        self.slider_move(x, update);
    }
    fn slider_move(&mut self, x: f32, update: &mut WidgetUpdate) {
        let Some(Gesture {
            target,
            kind: GestureKind::Slider {
                key: None, grab, ..
            },
            ..
        }) = &self.gesture
        else {
            return;
        };
        let target = target.clone();
        let grab = *grab;
        let WidgetSpec::Slider(s) = &self.controls[&target.widget].spec else {
            return;
        };
        let region = self.regions.iter().find(|r| r.target == target).unwrap();
        let (left, length) =
            crate::slider::track(region.rect, &self.theme.slider, s.value_label.is_some());
        if length > 0.0 {
            let value = s.domain.at_fraction((x - grab - left) / length);
            self.set_slider(&target, value, update);
        }
    }
    fn finish_slider(&mut self, g: Gesture, update: &mut WidgetUpdate) {
        if matches!(g.kind, GestureKind::Slider { changed: true, .. })
            && let Some(Control {
                spec: WidgetSpec::Slider(s),
                ..
            }) = self.controls.get(&g.target.widget)
        {
            update.emit(
                &g.target.widget,
                WidgetAction::SliderCommitted { value: s.value },
            );
        }
    }
    fn slider_key(
        &mut self,
        target: &WidgetTarget,
        key: Key,
        pressed: bool,
        update: &mut WidgetUpdate,
    ) {
        if !matches!(
            key,
            Key::Named(
                NamedKey::ArrowLeft
                    | NamedKey::ArrowRight
                    | NamedKey::ArrowUp
                    | NamedKey::ArrowDown
                    | NamedKey::PageUp
                    | NamedKey::PageDown
                    | NamedKey::Home
                    | NamedKey::End
            )
        ) {
            return;
        }
        update.status.consume = true;
        if !pressed {
            if self
                .gesture
                .as_ref()
                .is_some_and(|g| matches!(g.kind,GestureKind::Slider{key:Some(k),..} if k==key))
            {
                let g = self.gesture.take().unwrap();
                self.finish_slider(g, update);
                update.status.rerender = true;
            }
            return;
        }
        if self.gesture.as_ref().is_some_and(|g| !g.keyboard) {
            return;
        }
        if self
            .gesture
            .as_ref()
            .is_some_and(|g| matches!(g.kind,GestureKind::Slider{key:Some(k),..} if k!=key))
        {
            let g = self.gesture.take().unwrap();
            self.finish_slider(g, update);
        }
        let WidgetSpec::Slider(s) = &self.controls[&target.widget].spec else {
            return;
        };
        if self.gesture.is_none() {
            self.gesture = Some(Gesture {
                target: target.clone(),
                epoch: self.target_epoch(target).unwrap(),
                keyboard: true,
                inside: true,
                kind: GestureKind::Slider {
                    start: s.value,
                    last: s.value,
                    changed: false,
                    key: Some(key),
                    grab: 0.0,
                },
            });
        }
        let value = match key {
            Key::Named(NamedKey::Home) => s.domain.min(),
            Key::Named(NamedKey::End) => s.domain.max(),
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) => s.domain.advance(s.value, -1),
            Key::Named(NamedKey::PageDown) => s.domain.advance(s.value, -10),
            Key::Named(NamedKey::PageUp) => s.domain.advance(s.value, 10),
            _ => s.domain.advance(s.value, 1),
        };
        self.set_slider(target, value, update);
    }
}
