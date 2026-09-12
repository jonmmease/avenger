use crate::frame::{Region, visible};
use crate::{Rect, WidgetError, WidgetId, WidgetSpec, WidgetTarget, WidgetTheme};
use avenger_common::{cursor::CursorStyle, time::Instant};
use avenger_eventstream::{
    runtime::{KeyboardPolicy, RuntimeHostCommand as Command},
    scene::{ModifiersState, SceneGraphEvent as Event},
    stream::UpdateStatus,
    window::{ElementState, Key, MouseButton, NamedKey, TextInputEvent},
};
use avenger_geometry::rtree::SceneGraphRTree;
use std::{
    collections::BTreeMap,
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
    Activated,
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
}
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticValue {
    None,
    Checked(bool),
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
}
#[derive(Clone, Debug)]
pub(crate) struct Control {
    pub spec: WidgetSpec,
    pub epoch: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct Gesture {
    pub target: WidgetTarget,
    pub epoch: u64,
    pub keyboard: bool,
    pub inside: bool,
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
    pub fn focused(&self) -> Option<&WidgetTarget> {
        self.focused.as_ref()
    }
    /// Request visible logical focus, or blur. The caller delivers returned host effects.
    pub fn request_focus(
        &mut self,
        target: Option<WidgetTarget>,
        _now: Instant,
    ) -> Result<WidgetUpdate, WidgetError> {
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
                WidgetSemantic {
                    target: r.target.clone(),
                    role: match c.spec {
                        WidgetSpec::Button(_) => WidgetRole::Button,
                        WidgetSpec::Checkbox(_) => WidgetRole::Checkbox,
                    },
                    name: c
                        .spec
                        .options()
                        .semantic_name
                        .clone()
                        .unwrap_or_else(|| c.spec.label().into()),
                    enabled: c.spec.options().enabled,
                    focused: self.focused.as_ref() == Some(&r.target),
                    value: match &c.spec {
                        WidgetSpec::Button(_) => SemanticValue::None,
                        WidgetSpec::Checkbox(c) => SemanticValue::Checked(c.checked),
                    },
                    bounds: r.clip,
                }
            })
            .collect()
    }
    pub(crate) fn eligible(&self, t: &WidgetTarget) -> bool {
        self.controls
            .get(&t.widget)
            .is_some_and(|c| c.spec.options().enabled)
            && self
                .regions
                .iter()
                .any(|r| &r.target == t && visible(r.clip))
    }
    pub(crate) fn stops(&self) -> Vec<WidgetTarget> {
        self.order
            .iter()
            .map(|id| WidgetTarget::new(id.clone()))
            .filter(|t| self.eligible(t))
            .collect()
    }
    pub(crate) fn pressed(&self, t: &WidgetTarget) -> bool {
        self.gesture
            .as_ref()
            .is_some_and(|g| &g.target == t && g.inside)
    }
    fn cancel_gesture(&mut self, update: &mut WidgetUpdate) {
        if let Some(g) = self.gesture.take() {
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
            !self.eligible(&g.target) || self.controls[&g.target.widget].epoch != g.epoch
        }) {
            self.cancel_gesture(update);
        }
        if self.focused.as_ref().is_some_and(|t| !self.eligible(t)) {
            self.focus(None, false, update);
        }
        if self.remembered.as_ref().is_some_and(|t| !self.eligible(t)) {
            self.remembered = None;
        }
        if self.hovered.as_ref().is_some_and(|t| !self.eligible(t)) {
            self.hovered = None;
        }
    }
    fn focus(&mut self, target: Option<WidgetTarget>, visible: bool, update: &mut WidgetUpdate) {
        if self.focused == target {
            if self.focus_visible != visible {
                self.focus_visible = visible;
                update.status.rerender = true;
            }
            return;
        }
        self.cancel_gesture(update);
        if let Some(old) = self.focused.take() {
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
                WidgetSpec::Checkbox(_) => vec![NamedKey::Space, NamedKey::Escape],
            };
        }
        update.status.commands.push(Command::SetKeyboardPolicy {
            policy: Some(policy),
        });
    }
    fn activate(&mut self, target: &WidgetTarget, update: &mut WidgetUpdate) {
        if !self.eligible(target) {
            return;
        }
        match &mut self.controls.get_mut(&target.widget).unwrap().spec {
            WidgetSpec::Button(_) => update.emit(&target.widget, WidgetAction::Activated),
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
            if repeat {
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
        if modifiers.control || modifiers.alt || modifiers.meta {
            return;
        }
        match key {
            Key::Named(NamedKey::Escape) if pressed && self.gesture.is_some() => {
                self.cancel_gesture(update);
                update.status.consume = true;
            }
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
                        epoch: self.controls[&target.widget].epoch,
                        target,
                        keyboard: true,
                        inside: true,
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
        _now: Instant,
    ) -> Result<WidgetUpdate, WidgetError> {
        let mut update = WidgetUpdate::default();
        let hit = event
            .position()
            .and_then(|p| rtree.pick_top_mark_at_point(&p))
            .and_then(|m| self.regions.iter().find(|r| r.name == m.name))
            .map(|r| r.target.clone());
        match event {
            Event::MouseDown(e) if e.button == MouseButton::Left => {
                if let Some(target) = hit {
                    update.status.consume = true;
                    update.status.suppress_click = true;
                    if self.eligible(&target) {
                        self.focus(Some(target.clone()), false, &mut update);
                        self.gesture = Some(Gesture {
                            epoch: self.controls[&target.widget].epoch,
                            target,
                            keyboard: false,
                            inside: true,
                        });
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
                    update.status.cursor = Some(if hit.is_some() {
                        CursorStyle::Pointer
                    } else {
                        CursorStyle::Default
                    });
                }
                if let Some(g) = &mut self.gesture
                    && !g.keyboard
                {
                    let inside = hit.as_ref() == Some(&g.target);
                    update.status.rerender |= g.inside != inside;
                    g.inside = inside;
                    update.status.consume = true;
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
                    if hit.as_ref() == Some(&g.target) {
                        self.activate(&g.target, &mut update);
                    }
                }
            }
            Event::KeyPress(e) => self.key(e.key, true, e.repeat, e.modifiers, &mut update),
            Event::KeyRelease(e) => self.key(e.key, false, false, e.modifiers, &mut update),
            Event::TextInput { input, modifiers } => {
                if let TextInputEvent::Keyboard(k) = &input.event {
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
            Event::PointerCaptureLost => self.cancel_gesture(&mut update),
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
