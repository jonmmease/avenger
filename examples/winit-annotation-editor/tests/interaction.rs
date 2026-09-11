use avenger_app::{app::AppUpdate, app::AvengerApp};
use avenger_common::time::{Duration, Instant};
use avenger_eventstream::{
    runtime::{RuntimeHostCommand as Command, RuntimeTooltipUpdate, RuntimeWakeEvent},
    window::*,
};
use winit_annotation_editor::{
    make_app,
    state::{Drag, Sample, State},
};

struct Harness {
    app: AvengerApp<State>,
    time: Instant,
}
impl Harness {
    fn new() -> Self {
        Self {
            app: pollster::block_on(make_app(State::new(
                Sample::A,
                0,
                avenger_text::default_text_engine(),
            )))
            .unwrap(),
            time: Instant::now(),
        }
    }
    fn state(&mut self) -> &mut State {
        self.app.app_state_mut()
    }
    fn send(&mut self, event: WindowEvent) -> AppUpdate {
        pollster::block_on(self.app.update_with_status(&event, self.time)).unwrap()
    }
    fn advance(&mut self, ms: u64) {
        self.time += Duration::from_millis(ms);
    }
    fn move_to(&mut self, p: [f32; 2]) -> AppUpdate {
        self.send(WindowEvent::CursorMoved(WindowCursorMoved { position: p }))
    }
    fn down(&mut self) -> AppUpdate {
        self.send(WindowEvent::MouseInput(WindowMouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Left,
        }))
    }
    fn up(&mut self) -> AppUpdate {
        self.send(WindowEvent::MouseInput(WindowMouseInput {
            state: ElementState::Released,
            button: MouseButton::Left,
        }))
    }
    fn key(&mut self, key: Key, text: Option<&str>) -> AppUpdate {
        self.send(WindowEvent::KeyboardInput(WindowKeyboardInput {
            key,
            text: text.map(Into::into),
            state: ElementState::Pressed,
        }))
    }
    fn focus(&mut self) {
        let [x, y, _, _] = self.state().field();
        self.move_to([x + 16.0, y + 20.0]);
        self.down();
        self.up();
        assert!(self.state().focused);
    }
    fn all(&mut self) {
        let command = Key::Named(if cfg!(target_os = "macos") {
            NamedKey::Super
        } else {
            NamedKey::Control
        });
        self.key(command, None);
        self.key(Key::Character('a'), Some("a"));
        self.send(WindowEvent::KeyboardInput(WindowKeyboardInput {
            key: command,
            text: None,
            state: ElementState::Released,
        }));
    }
    fn replace(&mut self, text: &str) -> AppUpdate {
        self.all();
        self.send(WindowEvent::Clipboard(ClipboardEvent::Paste(text.into())))
    }
    fn wake(&mut self, wake: RuntimeWakeEvent) -> AppUpdate {
        self.send(WindowEvent::RuntimeWake(wake))
    }
}
fn wake(update: &AppUpdate, purpose: &str) -> RuntimeWakeEvent {
    update
        .status
        .commands
        .iter()
        .find_map(|c| match c {
            Command::RequestWakeup {
                key, generation, ..
            } if key.purpose.ends_with(purpose) => Some(RuntimeWakeEvent {
                key: key.clone(),
                generation: *generation,
            }),
            _ => None,
        })
        .expect("scheduled wake")
}
fn shows_tooltip(update: &AppUpdate) -> bool {
    update
        .status
        .commands
        .iter()
        .any(|c| matches!(c, Command::UpdateTooltip(RuntimeTooltipUpdate::Show(_))))
}

#[test]
fn draft_applies_after_silence_and_navigation_does_not_postpone_it() {
    let mut h = Harness::new();
    h.focus();
    let old = h.state().points[7].annotation.clone();
    let update = h.replace("Changed label");
    let apply = wake(&update, "apply");
    assert_eq!(h.state().editor.text(), "Changed label");
    assert_eq!(h.state().points[7].annotation, old);
    h.advance(200);
    let nav = h.key(Key::Named(NamedKey::ArrowLeft), None);
    assert!(!nav
        .status
        .commands
        .iter()
        .any(|c| matches!(c,Command::RequestWakeup{key,..} if key.purpose.ends_with("apply"))));
    assert!(!h.wake(apply.clone()).status.rebuild_geometry);
    h.advance(150);
    assert!(h.wake(apply).status.rebuild_geometry);
    assert_eq!(h.state().points[7].annotation, "Changed label");
    assert!(!h.state().pending());
}

#[test]
fn enter_flushes_escape_restores_and_old_wakes_cannot_apply() {
    let mut h = Harness::new();
    h.focus();
    let update = h.replace("Applied with Enter");
    let stale = wake(&update, "apply");
    h.key(Key::Named(NamedKey::Enter), None);
    assert!(!h.state().focused);
    assert_eq!(h.state().points[7].annotation, "Applied with Enter");
    h.advance(600);
    h.focus();
    let update = h.replace("Discard me");
    let blink = wake(&update, "blink");
    h.key(Key::Named(NamedKey::Escape), None);
    assert!(!h.state().focused);
    assert_eq!(h.state().editor.text(), "Applied with Enter");
    h.advance(1000);
    assert!(!h.wake(stale).status.rerender);
    assert!(!h.wake(blink).status.rerender);
    assert!(!h.state().caret_visible);
}

#[test]
fn composition_keeps_keyboard_payloads_out_and_commits_once() {
    let mut h = Harness::new();
    h.focus();
    h.all();
    let old = h.state().points[7].annotation.clone();
    let update = h.send(WindowEvent::Ime(ImeEvent::Preedit {
        text: "e\u{301}".into(),
        cursor: Some((0, 3)),
    }));
    assert!(update
        .status
        .commands
        .iter()
        .any(|c| matches!(c, Command::SetImeCursorArea { rect: Some(_) })));
    h.key(Key::Character('e'), Some("e\u{301}"));
    assert_eq!(h.state().editor.text(), "e\u{301}");
    assert_eq!(h.state().points[7].annotation, old);
    let commit = h.send(WindowEvent::Ime(ImeEvent::Commit("é".into())));
    let apply = wake(&commit, "apply");
    assert_eq!(h.state().editor.text(), "é");
    assert!(h.state().editor.compose_range().is_none());
    h.advance(350);
    h.wake(apply);
    assert_eq!(h.state().points[7].annotation, "é");
}

#[test]
fn focus_loss_restores_text_replaced_by_uncommitted_composition() {
    let mut h = Harness::new();
    h.focus();
    h.replace("Committed draft");
    h.all();
    h.send(WindowEvent::Ime(ImeEvent::Preedit {
        text: "unfinished".into(),
        cursor: Some((10, 10)),
    }));
    let update = h.send(WindowEvent::WindowFocused(false));
    assert!(!h.state().focused);
    assert!(h.state().editor.compose_range().is_none());
    assert_eq!(h.state().points[7].annotation, "Committed draft");
    assert!(update
        .status
        .commands
        .contains(&Command::SetImeAllowed { allowed: false }));
}

#[test]
fn clipboard_and_grapheme_deletion_use_the_editor() {
    let mut h = Harness::new();
    h.focus();
    h.replace("e\u{301} 👩‍💻");
    h.key(Key::Named(NamedKey::End), None);
    h.key(Key::Named(NamedKey::Backspace), None);
    assert_eq!(h.state().editor.text(), "e\u{301} ");
    assert!(h.state().error.is_none());
    h.all();
    let copied = h.send(WindowEvent::Clipboard(ClipboardEvent::Copy));
    assert!(copied.status.commands.contains(&Command::WriteClipboard {
        text: "e\u{301} ".into()
    }));
    h.send(WindowEvent::Clipboard(ClipboardEvent::Cut));
    assert_eq!(h.state().editor.text(), "");
    h.send(WindowEvent::Clipboard(ClipboardEvent::Paste(
        "pasted\nline".into(),
    )));
    assert_eq!(h.state().editor.text(), "pastedline");
}

#[test]
fn annotation_drag_captures_its_target_and_never_pans_the_plot() {
    let mut h = Harness::new();
    let p = h.state().point_position(7);
    let offset = h.state().points[7].offset;
    let start = [p[0] + offset[0] + 20.0, p[1] + offset[1] - 7.0];
    h.move_to(start);
    h.down();
    assert_eq!(h.state().drag, Some(Drag::Annotation));
    let end = [750.0, 272.0];
    let update = h.move_to(end);
    assert!(update.status.consume);
    h.up();
    assert_eq!(h.state().pan, [0.0; 2]);
    assert_eq!(h.state().selected, 7);
    assert_eq!(h.state().drag, None);
    assert_eq!(
        h.state().points[7].offset,
        [offset[0] + end[0] - start[0], offset[1] + end[1] - start[1]]
    );
}

#[test]
fn field_selection_and_background_pan_have_separate_ownership() {
    let mut h = Harness::new();
    h.focus();
    let [x, y, _, _] = h.state().field();
    h.move_to([x + 12.0, y + 20.0]);
    h.down();
    h.move_to([x + 120.0, y + 20.0]);
    h.up();
    assert!(!h.state().editor.selected_text().is_empty());
    assert_eq!(h.state().pan, [0.0; 2]);
    let [x, y, _, height] = h.state().plot();
    h.move_to([x + 5.0, y + height - 5.0]);
    h.down();
    assert_eq!(h.state().drag, Some(Drag::Plot));
    h.move_to([x + 25.0, y + height - 15.0]);
    h.up();
    assert_eq!(h.state().pan, [20.0, -10.0]);
}

#[test]
fn tooltip_wakes_move_and_cancel_without_rebuilding_the_scene() {
    let mut h = Harness::new();
    let p = h.state().point_position(3);
    let initial = h.move_to(p);
    let hover = wake(&initial, "hover");
    let builds = h.state().scene_builds;
    h.advance(400);
    let show = h.wake(hover.clone());
    assert!(shows_tooltip(&show));
    assert_eq!(h.state().scene_builds, builds);
    let moved = h.move_to([p[0] + 1.0, p[1]]);
    assert!(moved
        .status
        .commands
        .iter()
        .any(|c| matches!(c, Command::UpdateTooltip(RuntimeTooltipUpdate::Move { .. }))));
    assert_eq!(h.state().scene_builds, builds);
    h.send(WindowEvent::CursorLeft);
    h.advance(500);
    assert!(!shows_tooltip(&h.wake(hover)));
}

#[test]
fn selection_changes_flush_the_old_point_and_reject_its_timer() {
    let mut h = Harness::new();
    h.focus();
    let update = h.replace("Label for point eight");
    let old = wake(&update, "apply");
    let p = h.state().point_position(2);
    h.move_to(p);
    h.down();
    h.up();
    assert_eq!(h.state().selected, 2);
    assert_eq!(h.state().points[7].annotation, "Label for point eight");
    h.advance(800);
    h.wake(old);
    assert_eq!(h.state().points[2].annotation, "");
}

#[test]
fn focus_loss_clears_modifier_keys_and_resize_keeps_ime_inside_the_field() {
    let mut h = Harness::new();
    h.focus();
    let modifier = Key::Named(if cfg!(target_os = "macos") {
        NamedKey::Super
    } else {
        NamedKey::Control
    });
    h.key(modifier, None);
    h.send(WindowEvent::WindowFocused(false));
    h.send(WindowEvent::WindowFocused(true));
    h.advance(600);
    h.focus();
    let before = h.state().editor.text().to_string();
    h.key(Key::Character('z'), Some("z"));
    assert_ne!(h.state().editor.text(), before);
    let update = h.send(WindowEvent::WindowResize(WindowResizeEvent {
        size: [900.0, 600.0],
    }));
    let [x, y, w, height] = h.state().field();
    let area = update
        .status
        .commands
        .iter()
        .find_map(|c| match c {
            Command::SetImeCursorArea { rect } => *rect,
            _ => None,
        })
        .unwrap();
    assert!(area.x() >= x && area.x() < x + w);
    assert!(area.y() >= y && area.y() + area.height() <= y + height);
}

#[test]
fn an_old_application_wake_cannot_change_a_replacement() {
    let mut h = Harness::new();
    h.focus();
    let update = h.replace("old generation");
    let old = wake(&update, "apply");
    h.app = pollster::block_on(make_app(State::new(
        Sample::B,
        1,
        avenger_text::default_text_engine(),
    )))
    .unwrap();
    h.advance(500);
    assert!(!h.wake(old).status.rerender);
    assert_eq!(h.state().sample, Sample::B);
    assert!(!h.state().focused);
    assert_ne!(h.state().points[7].annotation, "old generation");
}

fn text_marks(
    marks: &[avenger_scenegraph::marks::mark::SceneMark],
) -> Vec<&avenger_scenegraph::marks::text::SceneTextMark> {
    use avenger_scenegraph::marks::mark::SceneMark;
    marks
        .iter()
        .flat_map(|mark| match mark {
            SceneMark::Text(mark) => vec![mark.as_ref()],
            SceneMark::Group(group) => text_marks(&group.marks),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn typst_source_stays_literal_in_the_field_and_typesets_in_the_annotation() {
    use avenger_text::types::TextSyntaxMode;
    let mut h = Harness::new();
    h.focus();
    let source = "*Distance* $sqrt(x^2+y^2)$";
    h.replace(source);
    h.key(Key::Named(NamedKey::Enter), None);
    assert_eq!(h.state().editor.text(), source);
    assert!(h.state().annotation_error.is_none());
    let marks = text_marks(&h.app.scene_graph().marks);
    let annotation = marks
        .iter()
        .find(|mark| mark.name == "annotation-7")
        .unwrap();
    assert_eq!(annotation.text, source.to_string().into());
    assert_eq!(annotation.text_syntax, TextSyntaxMode::TypstMarkup);
    assert!(marks
        .iter()
        .any(|mark| mark.text == source.to_string().into()
            && mark.text_syntax == TextSyntaxMode::Plain));
    let state = h.state();
    let plain = state.shaped_line().unwrap();
    let typeset = state
        .engine
        .measure_bounds(&winit_annotation_editor::state::annotation_config(source))
        .unwrap();
    assert!(plain.bounds.width > typeset.width + 10.0);
}

#[test]
fn invalid_markup_preserves_the_preview_and_enter_keeps_the_draft_editable() {
    let mut h = Harness::new();
    h.focus();
    let previous = h.state().points[7].annotation.clone();
    let update = h.replace("$sqrt(x");
    let stale = wake(&update, "apply");
    h.advance(350);
    let update = h.wake(stale.clone());
    assert!(update.scene_graph.is_some());
    assert!(!update.status.rebuild_geometry);
    assert_eq!(h.state().points[7].annotation, previous);
    assert_eq!(h.state().editor.text(), "$sqrt(x");
    assert!(h.state().annotation_error.is_some());
    h.key(Key::Named(NamedKey::ArrowLeft), None);
    assert!(h.state().annotation_error.is_some());
    h.key(Key::Named(NamedKey::Enter), None);
    assert!(h.state().focused);
    assert_eq!(h.state().points[7].annotation, previous);
    h.replace("_Fixed_ $sqrt(x)$");
    h.key(Key::Named(NamedKey::Enter), None);
    assert!(!h.state().focused);
    assert!(h.state().annotation_error.is_none());
    assert_eq!(h.state().points[7].annotation, "_Fixed_ $sqrt(x)$");
    h.advance(500);
    h.wake(stale);
    assert_eq!(h.state().points[7].annotation, "_Fixed_ $sqrt(x)$");
}

#[test]
fn escape_and_point_changes_discard_invalid_markup_without_losing_the_valid_label() {
    let mut h = Harness::new();
    h.focus();
    let previous = h.state().points[7].annotation.clone();
    h.replace("$sqrt(x");
    h.key(Key::Named(NamedKey::Enter), None);
    h.key(Key::Named(NamedKey::Escape), None);
    assert_eq!(h.state().editor.text(), previous);
    assert!(h.state().annotation_error.is_none());
    h.focus();
    h.replace("$sqrt(x");
    h.send(WindowEvent::WindowFocused(false));
    assert!(!h.state().focused);
    assert_eq!(h.state().editor.text(), "$sqrt(x");
    assert_eq!(h.state().points[7].annotation, previous);
    let point = h.state().point_position(2);
    h.send(WindowEvent::WindowFocused(true));
    h.move_to(point);
    h.down();
    h.up();
    assert_eq!(h.state().selected, 2);
    assert_eq!(h.state().editor.text(), "");
    assert!(h.state().annotation_error.is_none());
    assert_eq!(h.state().points[7].annotation, previous);
}

#[test]
fn blank_source_removes_the_annotation() {
    for source in ["", "   "] {
        let mut h = Harness::new();
        h.focus();
        h.all();
        h.key(Key::Named(NamedKey::Backspace), None);
        if !source.is_empty() {
            h.key(Key::Character(' '), Some(source));
        }
        h.key(Key::Named(NamedKey::Enter), None);
        assert_eq!(h.state().points[7].annotation, source);
        assert!(h.state().annotation_error.is_none());
        assert!(!text_marks(&h.app.scene_graph().marks)
            .iter()
            .any(|mark| mark.name == "annotation-7"));
    }
}
