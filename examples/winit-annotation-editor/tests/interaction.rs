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
fn clipboard_snapshot_tracks_selection_before_a_browser_copy_event() {
    let mut h = Harness::new();
    h.focus();
    h.all();
    let clipboard = h.state().clipboard_text.clone();
    assert_eq!(clipboard.lock().unwrap().1, h.state().editor.text());
    h.key(Key::Named(NamedKey::ArrowLeft), None);
    assert_eq!(clipboard.lock().unwrap().1, "");
    h.replace("New source");
    h.all();
    assert_eq!(clipboard.lock().unwrap().1, "New source");
    h.send(WindowEvent::Clipboard(ClipboardEvent::Cut));
    assert_eq!(clipboard.lock().unwrap().1, "");
}

#[test]
fn preparing_a_sample_does_not_replace_the_installed_clipboard_selection() {
    let mut h = Harness::new();
    h.focus();
    h.all();
    let clipboard = h.state().clipboard_text.clone();
    let expected = clipboard.lock().unwrap().clone();
    let mut next = State::new(Sample::B, 1, avenger_text::default_text_engine());
    next.clipboard_text = clipboard.clone();
    pollster::block_on(make_app(next)).unwrap();
    assert_eq!(*clipboard.lock().unwrap(), expected);
}

#[test]
fn select_all_uses_the_runtime_platform_shortcut() {
    for mac in [true, false] {
        let mut h = Harness::new();
        h.state().mac_shortcuts = mac;
        h.focus();
        h.key(
            Key::Named(if mac {
                NamedKey::Super
            } else {
                NamedKey::Control
            }),
            None,
        );
        h.key(Key::Character('a'), Some("a"));
        let source = h.state().editor.text().to_string();
        assert_eq!(h.state().editor.selected_text(), source);
    }
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
    let p = h.state().point_position(7).unwrap();
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
fn panning_moves_points_ticks_and_grid_together_without_snapping() {
    let mut h = Harness::new();
    let before = [axis_positions(&h, "x"), axis_positions(&h, "y")];
    let point_before = rendered_point_position(&h, "point-7");
    let [x, y, _, height] = h.state().plot();
    let start = [x + 5.0, y + height - 5.0];
    h.move_to(start);
    h.down();
    assert_eq!(h.state().drag, Some(Drag::Plot));

    for delta in [[0.25, -0.375], [-126.25, 91.5]] {
        h.move_to([start[0] + delta[0], start[1] + delta[1]]);
        let point = rendered_point_position(&h, "point-7");
        for (axis, name) in ["x", "y"].into_iter().enumerate() {
            let after = axis_positions(&h, name);
            assert!((point[axis] - point_before[axis] - delta[axis]).abs() < 0.001);
            for label in ["40", "60", "80"] {
                assert!((after[label] - before[axis][label] - delta[axis]).abs() < 0.001);
            }
        }
    }
    // Crossing a tick interval brings a new round value into each axis.
    for name in ["x", "y"] {
        let ticks = axis_positions(&h, name);
        assert!(ticks.contains_key("120"));
        assert!(!ticks.contains_key("0"));
        assert!(!ticks.contains_key("20"));
    }
    h.move_to(start);
    h.up();
    assert_eq!(axis_positions(&h, "x"), before[0]);
    assert_eq!(axis_positions(&h, "y"), before[1]);
    assert_eq!(rendered_point_position(&h, "point-7"), point_before);
}

#[test]
fn resized_plot_keeps_ticks_and_points_on_the_same_scales_after_panning() {
    let mut h = Harness::new();
    let [x, y, _, height] = h.state().plot();
    h.move_to([x + 5.0, y + height - 5.0]);
    h.down();
    h.move_to([x + 35.25, y + height - 20.5]);
    h.up();
    h.send(WindowEvent::WindowResize(WindowResizeEvent {
        size: [820.0, 560.0],
    }));
    let point = rendered_point_position(&h, "point-7");
    let values = h.state().points[7].position;
    for (axis, name) in ["x", "y"].into_iter().enumerate() {
        let ticks = axis_positions(&h, name);
        let fraction = (values[axis] - 60.0) / 20.0;
        let expected = ticks["60"] + fraction * (ticks["80"] - ticks["60"]);
        assert!((point[axis] - expected).abs() < 0.001);
    }
}

// Check the rendered labels against their grid lines, including viewport bounds.
fn axis_positions(h: &Harness, axis: &str) -> std::collections::BTreeMap<String, f32> {
    use avenger_scenegraph::marks::mark::SceneMark;
    let scene = h.app.scene_graph();
    let plot = scene
        .marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Group(group)
                if group
                    .marks
                    .iter()
                    .any(|m| matches!(m, SceneMark::Rect(r) if r.name == "plot")) =>
            {
                Some(group)
            }
            _ => None,
        })
        .unwrap();
    let labels: Vec<_> = text_marks(&scene.marks)
        .into_iter()
        .filter(|mark| mark.name == format!("{axis}-tick-label"))
        .collect();
    let grids: Vec<_> = plot
        .marks
        .iter()
        .filter_map(|mark| match mark {
            SceneMark::Rule(rule) if rule.name == format!("{axis}-grid") => Some(rule),
            _ => None,
        })
        .collect();
    assert!(!labels.is_empty());
    assert_eq!(labels.len(), grids.len());
    let avenger_scenegraph::marks::group::Clip::Rect {
        x,
        y,
        width,
        height,
    } = plot.clip
    else {
        panic!("plot must clip its grid and points");
    };
    labels
        .into_iter()
        .zip(grids)
        .map(|(label, grid)| {
            assert!(!label.interactive && !grid.interactive && grid.clip);
            let (position, start, end, lower, upper) = if axis == "x" {
                (
                    *label.x.first().unwrap(),
                    *grid.x.first().unwrap(),
                    *grid.x2.first().unwrap(),
                    x,
                    x + width,
                )
            } else {
                (
                    *label.y.first().unwrap(),
                    *grid.y.first().unwrap(),
                    *grid.y2.first().unwrap(),
                    y,
                    y + height,
                )
            };
            assert_eq!(position, start);
            assert_eq!(position, end);
            assert!(position >= lower - 0.001 && position <= upper + 0.001);
            (label.text.first().unwrap().clone(), position)
        })
        .collect()
}

fn rendered_point_position(h: &Harness, name: &str) -> [f32; 2] {
    use avenger_scenegraph::marks::mark::SceneMark;
    h.app
        .scene_graph()
        .marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Group(group) => group.marks.iter().find_map(|mark| match mark {
                SceneMark::Symbol(point) if point.name == name => {
                    Some([*point.x.first().unwrap(), *point.y.first().unwrap()])
                }
                _ => None,
            }),
            _ => None,
        })
        .unwrap()
}

#[test]
fn tooltip_wakes_move_and_cancel_without_rebuilding_the_scene() {
    let mut h = Harness::new();
    let p = h.state().point_position(3).unwrap();
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
    let p = h.state().point_position(2).unwrap();
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
    let point = h.state().point_position(2).unwrap();
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
