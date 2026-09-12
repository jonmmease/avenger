use avenger_common::time::{Duration, Instant};
use avenger_eventstream::{
    runtime::{InputSession, RuntimeHostCommand, RuntimeWakeEvent},
    scene::{ModifiersState, SceneGraphEvent},
    window::{
        ClipboardEvent, ElementState, ImeEvent, Key, NamedKey, SessionInputEvent, TextInputEvent,
        WindowKeyboardInput,
    },
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_text::text_edit::{Affinity, Cursor, SelectionState};
use avenger_widgets::prelude::*;

struct Field {
    id: &'static str,
    value: String,
    policy: TextCommitPolicy,
    enabled: bool,
    read_only: bool,
}
struct Rig {
    runtime: WidgetRuntime,
    fields: Vec<Field>,
    scene: SceneGraph,
    now: Instant,
}
impl Rig {
    fn new(policy: TextCommitPolicy) -> Self {
        let mut r = Self {
            runtime: WidgetRuntime::new().with_text_shortcuts(TextShortcuts::Control),
            fields: vec![
                Field {
                    id: "a",
                    value: "start".into(),
                    policy,
                    enabled: true,
                    read_only: false,
                },
                Field {
                    id: "b",
                    value: "other".into(),
                    policy: TextCommitPolicy::Immediate,
                    enabled: true,
                    read_only: false,
                },
            ],
            scene: SceneGraph {
                marks: vec![],
                width: 240.0,
                height: 160.0,
                origin: [0.0; 2],
            },
            now: Instant::now(),
        };
        r.build();
        r.runtime
            .request_focus(Some(WidgetTarget::new("a")), r.now)
            .unwrap();
        r
    }
    fn build(&mut self) -> WidgetUpdate {
        let specs: Vec<WidgetSpec> = self
            .fields
            .iter()
            .map(|f| {
                TextInput::new(f.id, &f.value)
                    .commit_policy(f.policy.clone())
                    .enabled(f.enabled)
                    .read_only(f.read_only)
                    .semantic_name(f.id)
                    .into()
            })
            .collect();
        let mut p = self
            .runtime
            .prepare(
                &specs,
                &WidgetTheme::light(),
                &avenger_text::default_text_engine(),
            )
            .unwrap();
        for (i, f) in self.fields.iter().enumerate() {
            p.place(
                f.id,
                Rect::new(10.0, 10.0 + i as f32 * 50.0, 200.0, 36.0),
                None,
            )
            .unwrap();
        }
        let frame = p.finish().unwrap();
        self.scene.marks = vec![frame.scene.clone().into()];
        self.runtime.install(frame).unwrap()
    }
    fn handle(&mut self, event: SceneGraphEvent) -> WidgetUpdate {
        self.now += Duration::from_millis(5);
        let mut u = self
            .runtime
            .handle(
                &event,
                &SceneGraphRTree::from_scene_graph(&self.scene),
                self.now,
            )
            .unwrap();
        for e in &u.events {
            if let WidgetAction::TextChanged { value } = &e.action
                && let Some(f) = self
                    .fields
                    .iter_mut()
                    .find(|f| WidgetId::from(f.id) == e.id)
            {
                f.value = value.clone();
            }
        }
        let installed = self.build();
        u.events.extend(installed.events);
        u.status.commands.extend(installed.status.commands);
        u
    }
    fn session(&self) -> InputSession {
        self.runtime.active_input_session().unwrap().clone()
    }
    fn input_as(
        &mut self,
        session: InputSession,
        event: TextInputEvent,
        modifiers: ModifiersState,
    ) -> WidgetUpdate {
        self.handle(SceneGraphEvent::TextInput {
            input: SessionInputEvent { session, event },
            modifiers,
        })
    }
    fn input(&mut self, event: TextInputEvent) -> WidgetUpdate {
        self.input_as(self.session(), event, ModifiersState::default())
    }
    fn key(&mut self, key: Key, text: Option<&str>, m: ModifiersState) -> WidgetUpdate {
        self.input_as(
            self.session(),
            TextInputEvent::Keyboard(WindowKeyboardInput {
                key,
                text: text.map(Into::into),
                state: ElementState::Pressed,
                repeat: false,
            }),
            m,
        )
    }
    fn named(&mut self, key: NamedKey) -> WidgetUpdate {
        self.key(Key::Named(key), None, ModifiersState::default())
    }
    fn type_text(&mut self, text: &str) -> WidgetUpdate {
        self.key(
            Key::Character(text.chars().next().unwrap()),
            Some(text),
            ModifiersState::default(),
        )
    }
    fn select_all(&mut self) {
        self.key(
            Key::Character('a'),
            None,
            ModifiersState {
                control: true,
                ..Default::default()
            },
        );
    }
}
fn actions(u: &WidgetUpdate) -> Vec<&WidgetAction> {
    u.events
        .iter()
        .filter(|e| !matches!(e.action, WidgetAction::FocusChanged { .. }))
        .map(|e| &e.action)
        .collect()
}
fn commit_wake(u: &WidgetUpdate) -> RuntimeWakeEvent {
    u.status
        .commands
        .iter()
        .find_map(|c| {
            if let RuntimeHostCommand::RequestWakeup {
                key, generation, ..
            } = c
            {
                (key.purpose == "commit").then(|| RuntimeWakeEvent {
                    key: key.clone(),
                    generation: *generation,
                })
            } else {
                None
            }
        })
        .unwrap()
}

#[test]
fn immediate_draft_commits_and_unchanged_enter_submission_keep_focus() {
    let mut r = Rig::new(TextCommitPolicy::Immediate);
    r.select_all();
    let u = r.type_text("Hello");
    assert_eq!(
        actions(&u),
        vec![
            &WidgetAction::TextChanged {
                value: "Hello".into()
            },
            &WidgetAction::TextCommitted {
                value: "Hello".into(),
                reason: TextCommitReason::Immediate
            }
        ]
    );
    assert_eq!(
        actions(&r.named(NamedKey::Enter)),
        vec![&WidgetAction::TextSubmitted {
            value: "Hello".into()
        }]
    );
    assert_eq!(r.runtime.focused(), Some(&WidgetTarget::new("a")));
    assert_eq!(
        actions(&r.named(NamedKey::Escape)),
        vec![&WidgetAction::TextCancelled {
            value: "Hello".into(),
            reason: TextCancelReason::Escape
        }]
    );
}
#[test]
fn debounce_rejects_stale_wakes_flushes_before_blur_and_cancels_on_disable() {
    let mut r = Rig::new(TextCommitPolicy::Debounced(Duration::from_millis(100)));
    r.select_all();
    let first = commit_wake(&r.type_text("A"));
    let second = commit_wake(&r.type_text("B"));
    r.now += Duration::from_millis(120);
    assert!(actions(&r.handle(SceneGraphEvent::RuntimeWake(first))).is_empty());
    assert_eq!(
        actions(&r.handle(SceneGraphEvent::RuntimeWake(second.clone()))),
        vec![&WidgetAction::TextCommitted {
            value: "AB".into(),
            reason: TextCommitReason::Debounced
        }]
    );
    assert!(actions(&r.handle(SceneGraphEvent::RuntimeWake(second))).is_empty());
    r.type_text("C");
    let u = r.named(NamedKey::Tab);
    let commit = u
        .events
        .iter()
        .position(|e| {
            matches!(
                e.action,
                WidgetAction::TextCommitted {
                    reason: TextCommitReason::Blur,
                    ..
                }
            )
        })
        .unwrap();
    let blur = u
        .events
        .iter()
        .position(|e| matches!(e.action, WidgetAction::FocusChanged { focused: false, .. }))
        .unwrap();
    assert!(commit < blur);
    r.runtime
        .request_focus(Some(WidgetTarget::new("a")), r.now)
        .unwrap();
    let wake = commit_wake(&r.type_text("D"));
    r.fields[0].enabled = false;
    let u = r.build();
    assert!(
        !u.events
            .iter()
            .any(|e| matches!(e.action, WidgetAction::TextCommitted { .. }))
    );
    assert!(r.runtime.focused().is_none());
    r.now += Duration::from_secs(1);
    assert!(actions(&r.handle(SceneGraphEvent::RuntimeWake(wake))).is_empty());
}
#[test]
fn escape_restores_baseline_without_committing_and_submit_orders_events() {
    let mut r = Rig::new(TextCommitPolicy::OnEnterOrBlur);
    r.select_all();
    r.type_text("draft");
    assert_eq!(
        actions(&r.named(NamedKey::Escape)),
        vec![
            &WidgetAction::TextChanged {
                value: "start".into()
            },
            &WidgetAction::TextCancelled {
                value: "start".into(),
                reason: TextCancelReason::Escape
            }
        ]
    );
    r.select_all();
    r.type_text("accepted");
    assert_eq!(
        actions(&r.named(NamedKey::Enter)),
        vec![
            &WidgetAction::TextCommitted {
                value: "accepted".into(),
                reason: TextCommitReason::Enter
            },
            &WidgetAction::TextSubmitted {
                value: "accepted".into()
            }
        ]
    );
}
#[test]
fn composition_cancellation_restores_selection_and_rejects_old_session() {
    let mut r = Rig::new(TextCommitPolicy::OnEnterOrBlur);
    r.select_all();
    let selection = r.runtime.text_selection("a").unwrap();
    let old = r.session();
    assert!(
        actions(&r.input(TextInputEvent::Ime(ImeEvent::Preedit {
            text: "é".into(),
            cursor: Some((2, 2))
        })))
        .is_empty()
    );
    assert!(actions(&r.named(NamedKey::Enter)).is_empty());
    assert_eq!(
        actions(&r.named(NamedKey::Escape)),
        vec![&WidgetAction::TextCancelled {
            value: "start".into(),
            reason: TextCancelReason::Composition
        }]
    );
    assert_eq!(r.runtime.text_selection("a"), Some(selection));
    assert_ne!(r.session(), old);
    assert!(
        actions(&r.input_as(
            old,
            TextInputEvent::Ime(ImeEvent::Commit("wrong".into())),
            ModifiersState::default()
        ))
        .is_empty()
    );
    assert_eq!(r.fields[0].value, "start");
}
#[test]
fn external_updates_during_composition_wait_and_latest_wins() {
    let mut r = Rig::new(TextCommitPolicy::Immediate);
    r.select_all();
    r.input(TextInputEvent::Ime(ImeEvent::Preedit {
        text: "x".into(),
        cursor: Some((1, 1)),
    }));
    r.fields[0].value = "first external".into();
    r.build();
    r.fields[0].value = "latest external".into();
    r.build();
    assert!(
        actions(&r.input(TextInputEvent::Ime(ImeEvent::Commit("composition".into())))).is_empty()
    );
    assert_eq!(r.fields[0].value, "latest external");
    r.select_all();
    let u = r.input(TextInputEvent::Clipboard(ClipboardEvent::Copy));
    assert!(
        u.status.commands.iter().any(
            |c| matches!(c,RuntimeHostCommand::WriteClipboard{text} if text=="latest external")
        )
    );
}
#[test]
fn tagged_clipboard_and_ime_cannot_cross_focus_or_reset() {
    let mut r = Rig::new(TextCommitPolicy::Immediate);
    let old = r.session();
    r.named(NamedKey::Tab);
    assert!(
        actions(&r.input_as(
            old.clone(),
            TextInputEvent::Clipboard(ClipboardEvent::Paste("late".into())),
            ModifiersState::default()
        ))
        .is_empty()
    );
    assert_eq!(r.fields[1].value, "other");
    r.runtime.reset_text("b", "fresh", r.now).unwrap();
    r.fields[1].value = "fresh".into();
    r.build();
    assert!(
        actions(&r.input_as(
            old,
            TextInputEvent::Ime(ImeEvent::Commit("late".into())),
            ModifiersState::default()
        ))
        .is_empty()
    );
    r.type_text("!");
    assert_eq!(r.fields[1].value, "fresh!");
}
#[test]
fn history_coalesces_typing_and_separates_paste_and_navigation() {
    let mut r = Rig::new(TextCommitPolicy::Immediate);
    r.select_all();
    r.type_text("a");
    r.type_text("b");
    r.type_text("c");
    let ctrl = ModifiersState {
        control: true,
        ..Default::default()
    };
    r.key(Key::Character('z'), None, ctrl);
    assert_eq!(r.fields[0].value, "start");
    r.key(Key::Character('y'), None, ctrl);
    assert_eq!(r.fields[0].value, "abc");
    r.input(TextInputEvent::Clipboard(ClipboardEvent::Paste(
        "\n\tx\u{2028}y\u{2029}".into(),
    )));
    assert_eq!(r.fields[0].value, "abcxy");
    r.key(Key::Character('z'), None, ctrl);
    assert_eq!(r.fields[0].value, "abc");
    r.named(NamedKey::ArrowLeft);
    r.type_text("Z");
    r.key(Key::Character('z'), None, ctrl);
    assert_eq!(r.fields[0].value, "abc");
    r.fields[0].value = "external".into();
    r.build();
    r.key(Key::Character('z'), None, ctrl);
    assert_eq!(r.fields[0].value, "external");
}
#[test]
fn readonly_allows_selection_and_copy_and_unicode_offsets_remain_graphemes() {
    let mut r = Rig::new(TextCommitPolicy::Immediate);
    r.fields[0].value = "e\u{301} 👨‍👩‍👧 אבג".into();
    r.fields[0].read_only = true;
    r.build();
    r.select_all();
    let u = r.input(TextInputEvent::Clipboard(ClipboardEvent::Cut));
    assert!(actions(&u).is_empty());
    assert!(
        u.status
            .commands
            .iter()
            .any(|c| matches!(c, RuntimeHostCommand::WriteClipboard { .. }))
    );
    assert!(actions(&r.type_text("no")).is_empty());
    assert!(
        actions(&r.input(TextInputEvent::Clipboard(ClipboardEvent::Paste(
            "no".into()
        ))))
        .is_empty()
    );
    r.runtime
        .set_text_selection(
            "a",
            SelectionState::collapsed(Cursor::new(1, Affinity::Downstream)),
            r.now,
        )
        .unwrap();
    assert_eq!(r.runtime.text_selection("a").unwrap().head.index, 0);
}

#[test]
fn composition_pauses_pending_debounce_and_cancellation_resumes_it() {
    let mut r = Rig::new(TextCommitPolicy::Debounced(Duration::from_millis(100)));
    let old = commit_wake(&r.type_text("A"));
    r.input(TextInputEvent::Ime(ImeEvent::Preedit {
        text: "é".into(),
        cursor: Some((2, 2)),
    }));
    r.now += Duration::from_secs(1);
    assert!(actions(&r.handle(SceneGraphEvent::RuntimeWake(old))).is_empty());
    let resumed = r.input(TextInputEvent::Ime(ImeEvent::Preedit {
        text: "".into(),
        cursor: None,
    }));
    let wake = commit_wake(&resumed);
    r.now += Duration::from_millis(120);
    assert_eq!(
        actions(&r.handle(SceneGraphEvent::RuntimeWake(wake))),
        vec![&WidgetAction::TextCommitted {
            value: "startA".into(),
            reason: TextCommitReason::Debounced
        }]
    );
}
#[test]
fn cancelling_composition_with_external_value_retires_the_late_commit() {
    let mut r = Rig::new(TextCommitPolicy::Immediate);
    r.select_all();
    let old = r.session();
    r.input(TextInputEvent::Ime(ImeEvent::Preedit {
        text: "x".into(),
        cursor: Some((1, 1)),
    }));
    r.fields[0].value = "external".into();
    r.build();
    r.input(TextInputEvent::Ime(ImeEvent::Preedit {
        text: "".into(),
        cursor: None,
    }));
    assert_ne!(r.session(), old);
    assert!(
        actions(&r.input_as(
            old,
            TextInputEvent::Ime(ImeEvent::Commit("late".into())),
            ModifiersState::default()
        ))
        .is_empty()
    );
    r.select_all();
    let u = r.input(TextInputEvent::Clipboard(ClipboardEvent::Copy));
    assert!(
        u.status
            .commands
            .iter()
            .any(|c| matches!(c,RuntimeHostCommand::WriteClipboard{text} if text=="external"))
    );
}
#[test]
fn readonly_stops_blink_and_programmatic_replacement_retires_clipboard() {
    let mut r = Rig::new(TextCommitPolicy::Immediate);
    let old = r.session();
    r.fields[0].read_only = true;
    let installed = r.build();
    assert_ne!(r.session(), old);
    assert!(
        !installed
            .status
            .commands
            .iter()
            .any(|c| matches!(c,RuntimeHostCommand::RequestWakeup{key,..} if key.purpose=="blink"))
    );
    let old = r.session();
    r.fields[0].read_only = false;
    r.fields[0].value = "changed".into();
    r.build();
    assert_ne!(r.session(), old);
    assert!(
        actions(&r.input_as(
            old,
            TextInputEvent::Clipboard(ClipboardEvent::Paste("late".into())),
            ModifiersState::default()
        ))
        .is_empty()
    );
}
#[test]
fn history_keeps_one_hundred_groups_and_new_edits_clear_redo() {
    let mut r = Rig::new(TextCommitPolicy::Immediate);
    r.select_all();
    r.input(TextInputEvent::Clipboard(ClipboardEvent::Cut));
    for _ in 0..105 {
        r.input(TextInputEvent::Clipboard(ClipboardEvent::Paste("x".into())));
    }
    let ctrl = ModifiersState {
        control: true,
        ..Default::default()
    };
    for _ in 0..105 {
        r.key(Key::Character('z'), None, ctrl);
    }
    assert_eq!(r.fields[0].value, "xxxxx");
    r.type_text("!");
    r.key(Key::Character('y'), None, ctrl);
    assert_eq!(r.fields[0].value, "xxxxx!");
}
#[test]
fn dragging_scrolls_when_stationary_and_release_retires_scroll_wakes() {
    use avenger_eventstream::{
        scene::{SceneCursorMovedEvent, SceneMouseDownEvent, SceneMouseUpEvent},
        window::MouseButton,
    };
    let mut r = Rig::new(TextCommitPolicy::Immediate);
    r.fields[0].value = "0123456789 ".repeat(40);
    r.build();
    r.runtime
        .set_text_selection(
            "a",
            SelectionState::collapsed(Cursor::new(0, Affinity::Downstream)),
            r.now,
        )
        .unwrap();
    r.build();
    r.handle(SceneGraphEvent::MouseDown(SceneMouseDownEvent {
        position: [20.0, 28.0],
        button: MouseButton::Left,
        mark_instance: None,
        modifiers: ModifiersState::default(),
    }));
    let update = r.handle(SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
        position: [240.0, 28.0],
        mark_instance: None,
        modifiers: ModifiersState::default(),
    }));
    let start = r.runtime.text_selection("a").unwrap().head.index;
    let wake = update
        .status
        .commands
        .iter()
        .find_map(|c| {
            if let RuntimeHostCommand::RequestWakeup {
                key, generation, ..
            } = c
            {
                (key.purpose == "scroll").then(|| RuntimeWakeEvent {
                    key: key.clone(),
                    generation: *generation,
                })
            } else {
                None
            }
        })
        .unwrap();
    r.now += Duration::from_millis(20);
    let update = r.handle(SceneGraphEvent::RuntimeWake(wake));
    assert!(r.runtime.text_selection("a").unwrap().head.index > start);
    let wake = update
        .status
        .commands
        .iter()
        .find_map(|c| {
            if let RuntimeHostCommand::RequestWakeup {
                key, generation, ..
            } = c
            {
                (key.purpose == "scroll").then(|| RuntimeWakeEvent {
                    key: key.clone(),
                    generation: *generation,
                })
            } else {
                None
            }
        })
        .unwrap();
    r.handle(SceneGraphEvent::MouseUp(SceneMouseUpEvent {
        position: [240.0, 28.0],
        button: MouseButton::Left,
        mark_instance: None,
        modifiers: ModifiersState::default(),
    }));
    let selected = r.runtime.text_selection("a");
    r.handle(SceneGraphEvent::RuntimeWake(wake));
    assert_eq!(r.runtime.text_selection("a"), selected);
}

#[test]
fn reset_retires_wakeup_keys_even_when_new_generations_restart() {
    let mut r = Rig::new(TextCommitPolicy::Debounced(Duration::from_millis(100)));
    let old = commit_wake(&r.type_text("old"));
    r.runtime.reset_text("a", "reset", r.now).unwrap();
    r.fields[0].value = "reset".into();
    r.build();
    let new = commit_wake(&r.type_text("new"));
    assert_ne!(old.key, new.key);
    r.now += Duration::from_secs(1);
    assert!(actions(&r.handle(SceneGraphEvent::RuntimeWake(old))).is_empty());
    assert_eq!(
        actions(&r.handle(SceneGraphEvent::RuntimeWake(new))),
        vec![&WidgetAction::TextCommitted {
            value: "resetnew".into(),
            reason: TextCommitReason::Debounced
        }]
    );
}
