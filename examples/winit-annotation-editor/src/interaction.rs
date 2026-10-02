use std::sync::Arc;

use async_trait::async_trait;
use avenger_common::{
    cursor::CursorStyle,
    time::{Duration, Instant},
};
use avenger_eventstream::{
    manager::EventStreamHandler,
    runtime::{
        RuntimeHostCommand as Command, RuntimeTooltipPresentation, RuntimeTooltipRow,
        RuntimeTooltipUpdate,
    },
    scene::{SceneGraphEvent as Event, SceneGraphEventType as Type},
    stream::{
        EventAdmission, EventStreamConfig, EventStreamContext, EventStreamFilter, UpdateStatus,
    },
    window::MouseButton,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_widgets::{TextCancelReason, WidgetAction, WidgetEvent};

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
    for (drag, prefix) in [(Drag::Annotation, "annotation-"), (Drag::Plot, "plot")] {
        registrations.push((
            EventStreamConfig {
                types: vec![
                    Type::CursorMoved,
                    Type::MouseUp,
                    Type::WindowFocused,
                    Type::PointerCaptureLost,
                ],
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
                        types: vec![Type::MouseUp, Type::WindowFocused, Type::PointerCaptureLost],
                        filter: Some(vec![EventStreamFilter::event(|event| {
                            matches!(event,Event::MouseUp(e) if e.button==MouseButton::Left)
                                || matches!(
                                    event,
                                    Event::WindowFocused(false) | Event::PointerCaptureLost
                                )
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
                Type::MouseUp,
                Type::CursorMoved,
                Type::MarkMouseLeave,
                Type::KeyRelease,
                Type::TextInput,
                Type::FocusEntered,
                Type::PointerCaptureLost,
                Type::KeyPress,
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
fn widget_events(
    state: &mut State,
    events: Vec<WidgetEvent>,
    time: Instant,
    status: &mut UpdateStatus,
) {
    for event in events {
        match event.action {
            WidgetAction::TextChanged { value } => {
                state.draft = value;
                state.annotation_error = None;
            }
            WidgetAction::TextCommitted { value, .. } => {
                apply_annotation(state, value, status);
            }
            WidgetAction::TextSubmitted { value } => {
                if apply_annotation(state, value, status) {
                    blur(state, time, status);
                }
            }
            WidgetAction::TextCancelled {
                reason: TextCancelReason::Escape | TextCancelReason::Composition,
                ..
            } => {
                // This demo's Escape restores the last accepted Typst label and leaves the field.
                reset_source(
                    state,
                    state.points[state.selected].annotation.clone(),
                    time,
                    status,
                );
                state.annotation_error = None;
                blur(state, time, status);
            }
            _ => {}
        }
    }
}
fn blur(state: &mut State, time: Instant, status: &mut UpdateStatus) {
    if let Ok(update) = state.widgets.request_focus(None, time) {
        *status = status.merge(&update.status);
        widget_events(state, update.events, time, status);
    }
}
fn reset_source(state: &mut State, value: String, time: Instant, status: &mut UpdateStatus) {
    state.draft = value.clone();
    match state.widgets.reset_text("source", value, time) {
        Ok(update) => *status = status.merge(&update.status),
        Err(error) => state.error = Some(error.to_string()),
    }
}
fn select(state: &mut State, selected: usize, time: Instant, status: &mut UpdateStatus) {
    if selected >= state.points.len() {
        return;
    }
    blur(state, time, status);
    state.selected = selected;
    reset_source(
        state,
        state.points[selected].annotation.clone(),
        time,
        status,
    );
    state.annotation_error = None;
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
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let time = now(context);
        let update = match state.widgets.handle(event, rtree, time) {
            Ok(update) => update,
            Err(error) => {
                state.error = Some(error.to_string());
                return accepted(true, false);
            }
        };
        let mut status = update.status;
        widget_events(state, update.events, time, &mut status);
        if matches!(event, Event::MouseDown(_)) {
            hide_hover(state, &mut status);
        }
        if status.consume {
            return status;
        }
        match event {
            Event::MouseDown(e) if e.button == MouseButton::Left => {
                state.pointer = e.position;
                let name = target(event);
                if let Some(i) = index(name, "point-") {
                    select(state, i, time, &mut status);
                } else if let Some(i) = index(name, "annotation-") {
                    select(state, i, time, &mut status);
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
                if state.drag.is_some() {
                    status.consume = true;
                    status.suppress_click = true;
                    status
                        .commands
                        .push(Command::SetPointerCapture { captured: true });
                }
            }
            Event::RuntimeWake(wake) => {
                if wake.key.attachment_epoch != state.generation
                    || wake.key.namespace != "annotation-editor"
                {
                    return status;
                }
                if wake.key == state.key("hover")
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
                    state.drag = None;
                }
                status.consume = false;
            }
            Event::PointerCaptureLost => {
                state.drag = None;
            }
            Event::WindowResize(e) => {
                state.size = e.size;
                status.rerender = true;
                status.rebuild_geometry = true;
            }
            Event::CanvasResize(e) => {
                state.size = e.size;
                status.rerender = true;
                status.rebuild_geometry = true;
            }
            Event::WindowCloseRequested => {
                hide_hover(state, &mut status);
                if let Some(reload) = state.reload.upgrade() {
                    reload.close();
                }
                status.consume = false;
            }
            _ => {}
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
            return if matches!(
                event,
                Event::MouseUp(_) | Event::WindowFocused(false) | Event::PointerCaptureLost
            ) {
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
        let mut status = accepted(true, true);
        match self.0 {
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
        status.cursor = Some(CursorStyle::Grabbing);
        if matches!(
            event,
            Event::MouseUp(_) | Event::WindowFocused(false) | Event::PointerCaptureLost
        ) {
            state.drag = None;
            status
                .commands
                .push(Command::SetPointerCapture { captured: false });
            status.cursor = Some(CursorStyle::Default);
        }
        if matches!(
            event,
            Event::WindowFocused(false) | Event::PointerCaptureLost
        ) {
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
        if name.starts_with("sample-") || point.is_some() {
            status.cursor = Some(CursorStyle::Pointer);
        } else if name == "plot" || name.starts_with("annotation-") {
            status.cursor = Some(CursorStyle::Grab);
        }
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
