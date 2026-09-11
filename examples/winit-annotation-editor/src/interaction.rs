use std::sync::Arc;

use async_trait::async_trait;
use avenger_common::{
    cursor::CursorStyle,
    time::{Duration, Instant},
};
use avenger_eventstream::{
    manager::EventStreamHandler,
    runtime::{
        LogicalRect, RuntimeHostCommand as Command, RuntimeTooltipPresentation, RuntimeTooltipRow,
        RuntimeTooltipUpdate,
    },
    scene::{SceneGraphEvent as Event, SceneGraphEventType as Type},
    stream::{
        EventAdmission, EventStreamConfig, EventStreamContext, EventStreamFilter, UpdateStatus,
    },
    window::{ClipboardEvent, ImeEvent, Key, MouseButton, NamedKey},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_text::text_edit::{cursor_rect_for_offset, Action, Motion, SingleLineEditor};

use crate::state::{annotation_config, Drag, Sample, State};

type Registration = (EventStreamConfig, Arc<dyn EventStreamHandler<State>>);

fn accepted(rerender: bool, geometry: bool) -> UpdateStatus {
    UpdateStatus {
        rerender,
        rebuild_geometry: geometry,
        consume: true,
        admission: Some(EventAdmission::Committed),
        ..Default::default()
    }
}
fn rejected() -> UpdateStatus {
    UpdateStatus {
        admission: Some(EventAdmission::Rejected),
        ..Default::default()
    }
}
fn target(event: &Event) -> &str {
    event.mark_instance().map_or("", |m| m.name.as_str())
}
fn index(name: &str, prefix: &str) -> Option<usize> {
    name.strip_prefix(prefix)?.parse().ok()
}
fn now(context: &EventStreamContext) -> Instant {
    context
        .current_event
        .as_ref()
        .expect("manager supplies event time")
        .instant
}

pub fn registrations() -> Vec<Registration> {
    let mut registrations: Vec<Registration> = Vec::new();
    // Arm before the input handler consumes the press. Movement and release use
    // the captured start event even when the pointer crosses another mark.
    for (drag, prefix) in [
        (Drag::Field, "field"),
        (Drag::Annotation, "annotation-"),
        (Drag::Plot, "plot"),
    ] {
        registrations.push((
            EventStreamConfig {
                types: vec![Type::CursorMoved, Type::MouseUp, Type::WindowFocused],
                between: Some((
                    Box::new(EventStreamConfig {
                        types: vec![Type::MouseDown],
                        filter: Some(vec![EventStreamFilter::event(move |event| {
                            matches!(event,Event::MouseDown(e) if e.button==MouseButton::Left)
                                && target(event).starts_with(prefix)
                        })]),
                        ..Default::default()
                    }),
                    Box::new(EventStreamConfig {
                        types: vec![Type::MouseUp, Type::WindowFocused],
                        filter: Some(vec![EventStreamFilter::event(|event| {
                            matches!(event,Event::MouseUp(e) if e.button==MouseButton::Left)
                                || matches!(event, Event::WindowFocused(false))
                        })]),
                        ..Default::default()
                    }),
                )),
                emit_between_end_event: true,
                ..Default::default()
            },
            Arc::new(DragHandler(drag)),
        ));
    }
    registrations.push((
        EventStreamConfig {
            types: vec![
                Type::MouseDown,
                Type::DoubleClick,
                Type::KeyPress,
                Type::Ime,
                Type::Clipboard,
                Type::RuntimeWake,
                Type::WindowFocused,
                Type::WindowResize,
                Type::CanvasResize,
                Type::WindowCloseRequested,
            ],
            ..Default::default()
        },
        Arc::new(InputHandler),
    ));
    registrations.push((
        EventStreamConfig {
            types: vec![Type::CursorMoved, Type::MarkMouseLeave],
            ..Default::default()
        },
        Arc::new(HoverHandler),
    ));
    registrations
}

pub fn ime_area(state: &mut State, status: &mut UpdateStatus) {
    if !state.focused {
        return;
    }
    state.keep_caret_visible();
    let head = state.editor.selection().head;
    if let Ok(line) = state.shaped_line() {
        let r = cursor_rect_for_offset(&line, head.index, head.affinity);
        let [x, y] = state.field_text_origin();
        status.commands.push(Command::SetImeCursorArea {
            rect: LogicalRect::new(x + r.x, y + r.y, 1.0, r.height),
        });
    }
}
fn blink(state: &mut State, time: Instant, status: &mut UpdateStatus) {
    status.rerender |= !state.caret_visible;
    state.caret_visible = true;
    state.blink_generation += 1;
    status.commands.push(Command::RequestWakeup {
        key: state.key("blink"),
        deadline: time + Duration::from_millis(500),
        generation: state.blink_generation,
    });
}
fn hide_hover(state: &mut State, status: &mut UpdateStatus) {
    status.commands.push(Command::CancelWakeup {
        key: state.key("hover"),
    });
    status
        .commands
        .push(Command::UpdateTooltip(RuntimeTooltipUpdate::Hide {
            owner: state.tooltip_owner().into(),
        }));
    state.hover_generation += 1;
    state.hover_point = None;
    state.hover_visible = false;
}
fn discard_composition(state: &mut State) {
    if state.editor.compose_range().is_some() {
        state.apply_action(Action::Commit(String::new()));
        if let Some((text, selection)) = state.composition_snapshot.take() {
            state.editor.restore_committed_state(text, selection);
        }
    }
}
// Keep incomplete source in the editor while the chart retains its last valid label.
fn apply_annotation(state: &mut State, text: String, status: &mut UpdateStatus) -> bool {
    let validation = if text.trim().is_empty() {
        Ok(())
    } else {
        state
            .engine
            .measure_bounds(&annotation_config(&text))
            .map(|_| ())
    };
    match validation {
        Ok(()) => {
            let changed = text != state.points[state.selected].annotation;
            state.points[state.selected].annotation = text;
            status.rebuild_geometry |= changed;
            let cleared_error = state.annotation_error.take().is_some();
            status.rerender |= changed || cleared_error;
            true
        }
        Err(error) => {
            state.annotation_error = Some(error.to_string());
            status.rerender = true;
            false
        }
    }
}
fn blur(state: &mut State, cancel: bool, status: &mut UpdateStatus) {
    if !state.focused {
        return;
    }
    discard_composition(state);
    let key = state.key("apply");
    status.commands.extend(state.debounce.cancel(&key).commands);
    if cancel {
        state.annotation_error = None;
        state
            .editor
            .replace_committed_text(state.points[state.selected].annotation.clone());
    } else {
        let text = state.editor.committed_text().into_string();
        apply_annotation(state, text, status);
    }
    status.commands.extend([
        Command::CancelWakeup {
            key: state.key("blink"),
        },
        Command::SetImeAllowed { allowed: false },
        Command::SetImeCursorArea { rect: None },
    ]);
    state.focused = false;
    state.drag = None;
    state.caret_visible = false;
    state.session += 1;
    status.rerender = true;
}
fn edit(state: &mut State, action: Action, time: Instant, status: &mut UpdateStatus) {
    let was_composing = state.editor.compose_range().is_some();
    let before = state.editor.committed_text().into_string();
    let changed = state.apply_action(action);
    status.rerender |= changed;
    let key = state.key("apply");
    if state.editor.compose_range().is_some() {
        status.commands.extend(state.debounce.cancel(&key).commands);
    } else if before != state.editor.committed_text().into_string()
        || (was_composing && state.pending())
    {
        status.commands.extend(
            state
                .debounce
                .submit(state.editor.committed_text().into_string(), time, &key)
                .commands,
        );
    }
    blink(state, time, status);
    ime_area(state, status);
}
fn select(state: &mut State, selected: usize, status: &mut UpdateStatus) {
    if selected >= state.points.len() {
        return;
    }
    blur(state, false, status);
    state.selected = selected;
    state.editor = SingleLineEditor::new(state.points[selected].annotation.clone());
    state.annotation_error = None;
    state.scroll = 0.0;
    status.rerender = true;
    status.rebuild_geometry = true;
}

struct InputHandler;
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<State> for InputHandler {
    async fn handle(&self, _: &Event, _: &mut State, _: &SceneGraphRTree) -> UpdateStatus {
        rejected()
    }
    async fn handle_with_context(
        &self,
        event: &Event,
        context: &EventStreamContext,
        state: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        let time = now(context);
        let mut status = accepted(false, false);
        match event {
            Event::MouseDown(e) if e.button == MouseButton::Left => {
                state.pointer = e.position;
                hide_hover(state, &mut status);
                let name = target(event);
                if name == "field" {
                    if !state.focused {
                        state.session += 1;
                        state.focused = true;
                        status
                            .commands
                            .push(Command::SetImeAllowed { allowed: true });
                    }
                    state.drag = Some(Drag::Field);
                    edit(
                        state,
                        Action::Click {
                            x: e.position[0] - state.field_text_origin()[0],
                        },
                        time,
                        &mut status,
                    );
                } else {
                    blur(state, false, &mut status);
                    if let Some(i) = index(name, "point-") {
                        select(state, i, &mut status);
                    } else if let Some(i) = index(name, "annotation-") {
                        select(state, i, &mut status);
                        state.drag = Some(Drag::Annotation);
                        state.drag_origin = state.points[state.selected].offset;
                    } else if name == "plot" {
                        state.drag = Some(Drag::Plot);
                        state.drag_origin = state.pan;
                    } else if name == "sample-a" || name == "sample-b" {
                        let sample = if name == "sample-a" {
                            Sample::A
                        } else {
                            Sample::B
                        };
                        if let Some(reload) = state.reload.upgrade() {
                            match reload.request(
                                sample,
                                state.size,
                                state.engine.clone(),
                                state.load_feedback.clone(),
                            ) {
                                Ok(()) => {
                                    state.loading = Some(sample);
                                    status.commands.push(Command::RequestWakeup {
                                        key: state.key("load"),
                                        deadline: time + Duration::from_millis(100),
                                        generation: state.generation,
                                    });
                                }
                                Err(error) => state.error = Some(error),
                            }
                        }
                        status.rerender = true;
                    }
                }
            }
            Event::DoubleClick(e) if target(event) == "field" && state.focused => {
                edit(
                    state,
                    Action::DoubleClick {
                        x: e.position[0] - state.field_text_origin()[0],
                    },
                    time,
                    &mut status,
                );
            }
            Event::KeyPress(e) if state.focused => {
                if e.key == Key::Named(NamedKey::Escape) {
                    blur(state, true, &mut status);
                    return status;
                }
                // Winit delivers composing text through IME events. Ignore key
                // payloads during composition so they cannot be inserted twice.
                if state.editor.compose_range().is_some() {
                    return status;
                }
                let command = if state.mac_shortcuts {
                    e.modifiers.meta
                } else {
                    e.modifiers.control
                };
                let word = if state.mac_shortcuts {
                    e.modifiers.alt
                } else {
                    e.modifiers.control
                };
                let action = match e.key {
                    Key::Named(NamedKey::Enter) => {
                        let text = state.editor.committed_text().into_string();
                        if !apply_annotation(state, text, &mut status) {
                            return status;
                        }
                        blur(state, false, &mut status);
                        return status;
                    }
                    Key::Named(NamedKey::Escape) => {
                        blur(state, true, &mut status);
                        return status;
                    }
                    Key::Named(NamedKey::Tab) => {
                        blur(state, false, &mut status);
                        return status;
                    }
                    Key::Character('a' | 'A') if command => Some(Action::SelectAll),
                    Key::Named(NamedKey::ArrowLeft) => Some(Action::Motion {
                        motion: if command && state.mac_shortcuts {
                            Motion::Start
                        } else if word {
                            Motion::WordLeft
                        } else {
                            Motion::Left
                        },
                        extend: e.modifiers.shift,
                    }),
                    Key::Named(NamedKey::ArrowRight) => Some(Action::Motion {
                        motion: if command && state.mac_shortcuts {
                            Motion::End
                        } else if word {
                            Motion::WordRight
                        } else {
                            Motion::Right
                        },
                        extend: e.modifiers.shift,
                    }),
                    Key::Named(NamedKey::Home) => Some(Action::Motion {
                        motion: Motion::Start,
                        extend: e.modifiers.shift,
                    }),
                    Key::Named(NamedKey::End) => Some(Action::Motion {
                        motion: Motion::End,
                        extend: e.modifiers.shift,
                    }),
                    Key::Named(NamedKey::Backspace) => Some(if word {
                        Action::DeleteWordBack
                    } else {
                        Action::Backspace
                    }),
                    Key::Named(NamedKey::Delete) => Some(if word {
                        Action::DeleteWordForward
                    } else {
                        Action::Delete
                    }),
                    _ if !command => e
                        .text
                        .as_ref()
                        .filter(|text| !text.chars().any(char::is_control))
                        .map(|text| Action::InsertText(text.to_string())),
                    _ => None,
                };
                if let Some(action) = action {
                    edit(state, action, time, &mut status);
                }
            }
            Event::Ime(ime) if state.focused => match ime {
                ImeEvent::Preedit { text, cursor } => {
                    if state.editor.compose_range().is_none() && !text.is_empty() {
                        state.composition_snapshot =
                            Some((state.editor.text().to_string(), state.editor.selection()));
                    }
                    edit(
                        state,
                        Action::Preedit {
                            text: text.to_string(),
                            cursor: *cursor,
                        },
                        time,
                        &mut status,
                    );
                }
                ImeEvent::Commit(text) => {
                    edit(state, Action::Commit(text.to_string()), time, &mut status);
                    state.composition_snapshot = None;
                }
                ImeEvent::Disabled => {
                    discard_composition(state);
                    if state.pending() {
                        let key = state.key("apply");
                        status.commands.extend(
                            state
                                .debounce
                                .submit(state.editor.committed_text().into_string(), time, &key)
                                .commands,
                        );
                    }
                    status.rerender = true;
                    ime_area(state, &mut status);
                }
                ImeEvent::Enabled => {}
            },
            Event::Clipboard(action) if state.focused && state.editor.compose_range().is_none() => {
                match action {
                    ClipboardEvent::Copy | ClipboardEvent::Cut => {
                        let text = state.editor.selected_text().to_string();
                        if !text.is_empty() {
                            status.commands.push(Command::WriteClipboard { text });
                            if matches!(action, ClipboardEvent::Cut) {
                                edit(state, Action::Backspace, time, &mut status);
                            }
                        }
                    }
                    ClipboardEvent::Paste(text) => edit(
                        state,
                        Action::InsertText(text.to_string()),
                        time,
                        &mut status,
                    ),
                }
            }
            Event::RuntimeWake(wake) => {
                if wake.key.attachment_epoch != state.generation
                    || wake.key.namespace != "annotation-editor"
                {
                    return rejected();
                }
                let update = state.debounce.handle_wakeup(wake, time);
                status.commands.extend(update.commands);
                if let Some(text) = update.commit {
                    apply_annotation(state, text, &mut status);
                }
                if wake.key == state.key("blink")
                    && wake.generation == state.blink_generation
                    && state.focused
                {
                    state.caret_visible = !state.caret_visible;
                    state.blink_generation += 1;
                    status.rerender = true;
                    status.commands.push(Command::RequestWakeup {
                        key: wake.key.clone(),
                        deadline: time + Duration::from_millis(500),
                        generation: state.blink_generation,
                    });
                } else if wake.key == state.key("hover")
                    && wake.generation == state.hover_generation
                    && state.window_focused
                    && state.drag.is_none()
                {
                    if let Some(i) = state.hover_point {
                        let p = &state.points[i];
                        state.hover_visible = true;
                        status
                            .commands
                            .push(Command::UpdateTooltip(RuntimeTooltipUpdate::Show(
                                RuntimeTooltipPresentation {
                                    owner: state.tooltip_owner().into(),
                                    anchor: state.pointer,
                                    offset: [14.0, 18.0],
                                    rows: vec![
                                        RuntimeTooltipRow {
                                            label: "Point".into(),
                                            value: p.name.clone(),
                                        },
                                        RuntimeTooltipRow {
                                            label: "Position".into(),
                                            value: format!(
                                                "{:.0}, {:.0}",
                                                p.position[0], p.position[1]
                                            ),
                                        },
                                        RuntimeTooltipRow {
                                            label: "Annotation".into(),
                                            value: if p.annotation.is_empty() {
                                                "—".into()
                                            } else {
                                                p.annotation.clone()
                                            },
                                        },
                                    ],
                                    style: Default::default(),
                                },
                            )));
                    }
                } else if wake.key == state.key("load") && state.loading.is_some() {
                    let result = state.load_feedback.lock().expect("load feedback").clone();
                    match result {
                        Some(result) => {
                            state.loading = None;
                            state.error = result.err();
                            status.rerender = true;
                        }
                        None => status.commands.push(Command::RequestWakeup {
                            key: wake.key.clone(),
                            deadline: time + Duration::from_millis(100),
                            generation: state.generation,
                        }),
                    }
                }
            }
            Event::WindowFocused(focused) => {
                state.window_focused = *focused;
                if !focused {
                    hide_hover(state, &mut status);
                    blur(state, false, &mut status);
                    state.drag = None;
                }
                // Focus lifecycle must reach all subscribers, including gesture cleanup.
                status.consume = false;
            }
            Event::WindowResize(e) => {
                state.size = e.size;
                status.rerender = true;
                status.rebuild_geometry = true;
                ime_area(state, &mut status);
            }
            Event::CanvasResize(e) => {
                state.size = e.size;
                status.rerender = true;
                status.rebuild_geometry = true;
                ime_area(state, &mut status);
            }
            Event::WindowCloseRequested => {
                hide_hover(state, &mut status);
                blur(state, false, &mut status);
                if let Some(reload) = state.reload.upgrade() {
                    reload.close();
                }
                status.consume = false;
            }
            _ => return rejected(),
        }
        status
    }
}

struct DragHandler(Drag);
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<State> for DragHandler {
    async fn handle(&self, _: &Event, _: &mut State, _: &SceneGraphRTree) -> UpdateStatus {
        rejected()
    }
    async fn handle_with_context(
        &self,
        event: &Event,
        context: &EventStreamContext,
        state: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        if state.drag != Some(self.0) {
            return if matches!(event, Event::MouseUp(_) | Event::WindowFocused(false)) {
                UpdateStatus {
                    admission: Some(EventAdmission::Committed),
                    ..Default::default()
                }
            } else {
                rejected()
            };
        }
        let Some(start) = context
            .start_event
            .as_ref()
            .and_then(|e| e.event.position())
        else {
            return rejected();
        };
        let position = event.position().unwrap_or(state.pointer);
        state.pointer = position;
        let delta = [position[0] - start[0], position[1] - start[1]];
        let mut status = accepted(true, self.0 != Drag::Field);
        match self.0 {
            Drag::Field => {
                state.apply_action(Action::Drag {
                    x: position[0] - state.field_text_origin()[0],
                });
                blink(state, now(context), &mut status);
                ime_area(state, &mut status);
            }
            Drag::Annotation => {
                state.points[state.selected].offset = [
                    state.drag_origin[0] + delta[0],
                    state.drag_origin[1] + delta[1],
                ]
            }
            Drag::Plot => {
                state.pan = [
                    state.drag_origin[0] + delta[0],
                    state.drag_origin[1] + delta[1],
                ]
            }
        }
        status.cursor = Some(if self.0 == Drag::Field {
            CursorStyle::Text
        } else {
            CursorStyle::Grabbing
        });
        if matches!(event, Event::MouseUp(_) | Event::WindowFocused(false)) {
            state.drag = None;
            status.cursor = Some(CursorStyle::Default);
        }
        if matches!(event, Event::WindowFocused(false)) {
            status.consume = false;
        }
        status
    }
}

struct HoverHandler;
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<State> for HoverHandler {
    async fn handle(&self, _: &Event, _: &mut State, _: &SceneGraphRTree) -> UpdateStatus {
        rejected()
    }
    async fn handle_with_context(
        &self,
        event: &Event,
        context: &EventStreamContext,
        state: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut status = UpdateStatus::default();
        if matches!(event, Event::MouseLeave(_)) {
            hide_hover(state, &mut status);
            return status;
        }
        let Event::CursorMoved(e) = event else {
            return status;
        };
        state.pointer = e.position;
        if state.drag.is_some() {
            return status;
        }
        let name = target(event);
        let point = index(name, "point-");
        status.cursor = Some(if name == "field" {
            CursorStyle::Text
        } else if name.starts_with("sample-") || point.is_some() {
            CursorStyle::Pointer
        } else if name == "plot" || name.starts_with("annotation-") {
            CursorStyle::Grab
        } else {
            CursorStyle::Default
        });
        if state.hover_point != point {
            hide_hover(state, &mut status);
            state.hover_point = point;
            if point.is_some() {
                status.commands.push(Command::RequestWakeup {
                    key: state.key("hover"),
                    deadline: now(context) + Duration::from_millis(400),
                    generation: state.hover_generation,
                });
            }
        } else if state.hover_visible {
            status
                .commands
                .push(Command::UpdateTooltip(RuntimeTooltipUpdate::Move {
                    owner: state.tooltip_owner().into(),
                    anchor: e.position,
                }));
        }
        status
    }
}
