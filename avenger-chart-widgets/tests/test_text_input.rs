use std::sync::Arc;

use avenger_chart::prelude::{
    Cartesian, Chart, ChromePosition, EvaluationRequest, InMemoryNativeWidgetInstanceStore,
    NativeWidgetDocumentId, NativeWidgetEvent, NativeWidgetEventRoute, NativeWidgetHostTransform,
    NativeWidgetPlacementExt, NativeWidgetPlotId, NativeWidgetRegistry,
    NativeWidgetRuntimeResources, PlotSession, PlotSessionOptions, Theme,
};
use avenger_chart_widgets::{TextCommit, TextInput, TextInputFactory};
use avenger_common::time::{Duration, Instant};
use avenger_eventstream::{
    runtime::{RuntimeHostCommand, RuntimeWakeEvent},
    scene::{
        ModifiersState, SceneGraphEvent, SceneKeyPressEvent, SceneMouseDownEvent, SceneMouseUpEvent,
    },
    window::{ClipboardEvent, ImeEvent, Key, MouseButton, NamedKey},
};
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, rect::SceneRectMark, rule::SceneRuleMark,
    text::SceneTextMark,
};
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

struct Harness {
    session: PlotSession,
    route: NativeWidgetEventRoute,
    frame_size: [f32; 2],
    now: Instant,
    resources: NativeWidgetRuntimeResources,
    plot_id: NativeWidgetPlotId,
}

impl Harness {
    async fn new(
        widget: TextInput,
        theme: Option<Theme>,
    ) -> (Self, avenger_chart::render::EvaluatedPlot) {
        let ctx = Arc::new(SessionContext::new());
        let mut chart = Chart::<Cartesian>::new().plot_size(240.0, 120.0);
        if let Some(theme) = theme {
            chart = chart.theme(theme);
        }
        let compiled = chart
            .native_widget(widget.position(ChromePosition::Top))
            .compile(ctx.as_ref())
            .await
            .unwrap();
        let compiled: avenger_chart::plot::CompiledPlot =
            bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();
        let registry = Arc::new(
            NativeWidgetRegistry::new()
                .with_factory(TextInputFactory)
                .unwrap(),
        );
        let resources = NativeWidgetRuntimeResources::new(
            registry,
            Arc::new(InMemoryNativeWidgetInstanceStore::new()),
            NativeWidgetDocumentId::new(),
        );
        let mut session = Arc::new(compiled).instantiate(ctx);
        let plot_id = NativeWidgetPlotId::from_member_path("text-input-test");
        session.set_options(PlotSessionOptions::from_native_widget_resources(
            &resources,
            plot_id.clone(),
        ));
        let now = Instant::now();
        let evaluated = session
            .evaluate(EvaluationRequest::new().at(now))
            .await
            .unwrap();
        let attachment = evaluated.native_widgets.by_widget_id["search"].clone();
        let frame = evaluated.widget_frames.by_widget_id["search"].clone();
        (
            Self {
                session,
                route: NativeWidgetEventRoute {
                    key: attachment.key,
                    epoch: attachment.epoch,
                },
                frame_size: [frame.bounds.width, frame.bounds.height],
                now,
                resources,
                plot_id,
            },
            evaluated,
        )
    }

    fn dispatch_at(
        &mut self,
        event: SceneGraphEvent,
        hit_part: Option<&str>,
        current: Option<[f32; 2]>,
        now: Instant,
    ) -> avenger_chart::prelude::NativeWidgetDispatchOutcome {
        self.session
            .dispatch_native_widget_event(
                &self.route,
                &NativeWidgetEvent {
                    event,
                    hit_part: hit_part.map(str::to_string),
                    current,
                    start: current,
                    previous: None,
                    wheel_delta: None,
                    frame_size: self.frame_size,
                },
                NativeWidgetHostTransform::default(),
                now,
                self.session.base_font_size(),
            )
            .unwrap()
    }

    fn dispatch(
        &mut self,
        event: SceneGraphEvent,
        hit_part: Option<&str>,
        current: Option<[f32; 2]>,
    ) -> avenger_chart::prelude::NativeWidgetDispatchOutcome {
        self.dispatch_at(event, hit_part, current, self.now)
    }

    async fn apply_and_evaluate(
        &mut self,
        outcome: &avenger_chart::prelude::NativeWidgetDispatchOutcome,
        now: Instant,
    ) -> avenger_chart::render::EvaluatedPlot {
        let patch = outcome
            .param_assignments
            .iter()
            .filter(|assignment| assignment.owner_path.is_empty())
            .map(|assignment| (assignment.name.clone(), assignment.value.clone()))
            .collect::<IndexMap<_, _>>();
        if !patch.is_empty() {
            self.session.apply_param_patch(patch);
        }
        self.session
            .evaluate(EvaluationRequest::new().at(now))
            .await
            .unwrap()
    }

    fn focus(&mut self) -> avenger_chart::prelude::NativeWidgetDispatchOutcome {
        self.dispatch(
            mouse_down([self.frame_size[0] - 8.0, self.frame_size[1] * 0.5]),
            Some("box"),
            Some([self.frame_size[0] - 8.0, self.frame_size[1] * 0.5]),
        )
    }
}

fn mouse_down(position: [f32; 2]) -> SceneGraphEvent {
    SceneGraphEvent::MouseDown(SceneMouseDownEvent {
        position,
        button: MouseButton::Left,
        mark_instance: None,
        modifiers: ModifiersState::default(),
    })
}

fn mouse_up(position: [f32; 2]) -> SceneGraphEvent {
    SceneGraphEvent::MouseUp(SceneMouseUpEvent {
        position,
        button: MouseButton::Left,
        mark_instance: None,
        modifiers: ModifiersState::default(),
    })
}

fn key(key: Key, text: Option<&str>, modifiers: ModifiersState) -> SceneGraphEvent {
    SceneGraphEvent::KeyPress(SceneKeyPressEvent {
        position: [0.0, 0.0],
        key,
        text: text.map(Into::into),
        mark_instance: None,
        modifiers,
    })
}

fn command_modifier() -> ModifiersState {
    if cfg!(target_os = "macos") {
        ModifiersState {
            meta: true,
            ..Default::default()
        }
    } else {
        ModifiersState {
            control: true,
            ..Default::default()
        }
    }
}

fn find_group<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneGroup> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Group(group) if group.name == name => Some(group),
        SceneMark::Group(group) => find_group(&group.marks, name),
        _ => None,
    })
}

fn find_text<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneTextMark> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Text(text) if text.name == name => Some(text.as_ref()),
        SceneMark::Group(group) => find_text(&group.marks, name),
        _ => None,
    })
}

fn find_rect<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneRectMark> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Rect(rect) if rect.name == name => Some(rect),
        SceneMark::Group(group) => find_rect(&group.marks, name),
        _ => None,
    })
}

fn find_rule<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneRuleMark> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Rule(rule) if rule.name == name => Some(rule),
        SceneMark::Group(group) => find_rule(&group.marks, name),
        _ => None,
    })
}

fn text_value(scene: &avenger_chart::render::EvaluatedPlot) -> String {
    let group = find_group(&scene.scene_graph.marks, "search").unwrap();
    let text = find_text(&group.marks, "text").unwrap();
    text.text.as_vec(text.len as usize, None)[0].clone()
}

fn mark_name(mark: &SceneMark) -> &str {
    match mark {
        SceneMark::Arc(mark) => &mark.name,
        SceneMark::Area(mark) => &mark.name,
        SceneMark::Group(mark) => &mark.name,
        SceneMark::Image(mark) => &mark.name,
        SceneMark::Line(mark) => &mark.name,
        SceneMark::Path(mark) => &mark.name,
        SceneMark::Rect(mark) => &mark.name,
        SceneMark::Rule(mark) => &mark.name,
        SceneMark::Symbol(mark) => &mark.name,
        SceneMark::Text(mark) => &mark.name,
        SceneMark::Trail(mark) => &mark.name,
        SceneMark::WarpedImage(mark) => &mark.name,
    }
}

#[tokio::test]
async fn text_input_round_trips_and_renders_stable_parts() {
    let widget = TextInput::new("search")
        .placeholder("Filter…")
        .initial_value("horse");
    let _ = widget.cursor_position();
    let _ = widget.selected_text();
    let (harness, evaluated) = Harness::new(widget, None).await;

    assert_eq!(
        harness.session.params().get("search__value"),
        Some(&ScalarValue::Utf8(Some("horse".to_string())))
    );
    assert_eq!(
        harness.session.params().get("search__cursor"),
        Some(&ScalarValue::UInt64(Some(0)))
    );
    assert_eq!(harness.frame_size[1], 32.0);
    let group = find_group(&evaluated.scene_graph.marks, "search").unwrap();
    assert_eq!(
        group.marks.iter().map(mark_name).collect::<Vec<_>>(),
        [
            "rule_mark",
            "box",
            "focus-ring",
            "selection",
            "text",
            "placeholder",
            "preedit",
            "caret"
        ]
    );
    assert_eq!(text_value(&evaluated), "horse");
}

#[tokio::test]
async fn on_change_replaces_deadline_and_survives_opt_in_state_rerenders() {
    let widget = TextInput::new("search").initial_value("a").debounce(150);
    let _ = widget.cursor_position();
    let _ = widget.selected_text();
    let (mut harness, _) = Harness::new(widget, None).await;
    harness.focus();

    let first_at = harness.now + Duration::from_millis(10);
    let first = harness.dispatch_at(
        key(Key::Character('b'), Some("b"), ModifiersState::default()),
        None,
        None,
        first_at,
    );
    assert!(
        !first
            .param_assignments
            .iter()
            .any(|assignment| assignment.name == "search__value")
    );
    let first_wake = first
        .commands
        .iter()
        .find_map(|command| match command {
            RuntimeHostCommand::RequestWakeup {
                key, generation, ..
            } => Some(RuntimeWakeEvent {
                key: key.clone(),
                generation: *generation,
            }),
            _ => None,
        })
        .unwrap();
    harness.apply_and_evaluate(&first, first_at).await;
    assert_eq!(
        harness.session.params()["search__cursor"],
        ScalarValue::UInt64(Some(2))
    );
    assert_eq!(
        harness.session.params()["search__value"],
        ScalarValue::Utf8(Some("a".into()))
    );

    let second_at = harness.now + Duration::from_millis(60);
    let second = harness.dispatch_at(
        key(Key::Character('c'), Some("c"), ModifiersState::default()),
        None,
        None,
        second_at,
    );
    let second_wake = second
        .commands
        .iter()
        .find_map(|command| match command {
            RuntimeHostCommand::RequestWakeup {
                key, generation, ..
            } => Some(RuntimeWakeEvent {
                key: key.clone(),
                generation: *generation,
            }),
            _ => None,
        })
        .unwrap();
    assert!(second_wake.generation > first_wake.generation);
    harness.apply_and_evaluate(&second, second_at).await;

    let stale = harness.dispatch_at(
        SceneGraphEvent::RuntimeWake(first_wake),
        None,
        None,
        harness.now + Duration::from_millis(170),
    );
    assert!(
        stale.param_assignments.is_empty(),
        "stale wake assignments: {:?}",
        stale.param_assignments
    );

    let ready = harness.dispatch_at(
        SceneGraphEvent::RuntimeWake(second_wake),
        None,
        None,
        harness.now + Duration::from_millis(220),
    );
    assert_eq!(
        ready
            .param_assignments
            .iter()
            .find(|assignment| assignment.name == "search__value")
            .map(|assignment| &assignment.value),
        Some(&ScalarValue::Utf8(Some("abc".into())))
    );
    harness
        .apply_and_evaluate(&ready, harness.now + Duration::from_millis(220))
        .await;

    // Matching self-acknowledgement did not discard the batched undo point.
    let undo = harness.dispatch_at(
        key(Key::Character('z'), None, command_modifier()),
        None,
        None,
        harness.now + Duration::from_millis(230),
    );
    let scene = harness
        .apply_and_evaluate(&undo, harness.now + Duration::from_millis(230))
        .await;
    assert_eq!(text_value(&scene), "a");
}

#[tokio::test]
async fn selection_survives_scene_rerender_and_replaces_on_next_edit() {
    let widget = TextInput::new("search").initial_value("old").debounce(150);
    let (mut harness, _) = Harness::new(widget, None).await;
    harness.focus();

    let select_all = harness.dispatch(
        key(Key::Character('a'), None, command_modifier()),
        None,
        None,
    );
    let selected = harness
        .apply_and_evaluate(&select_all, harness.now + Duration::from_millis(1))
        .await;
    let group = find_group(&selected.scene_graph.marks, "search").unwrap();
    assert!(find_rect(&group.marks, "selection").unwrap().len > 0);

    let typed = harness.dispatch_at(
        key(Key::Character('x'), Some("x"), ModifiersState::default()),
        None,
        None,
        harness.now + Duration::from_millis(2),
    );
    let replaced = harness
        .apply_and_evaluate(&typed, harness.now + Duration::from_millis(2))
        .await;
    assert_eq!(text_value(&replaced), "x");
}

#[tokio::test]
async fn enter_or_blur_keeps_text_local_but_publishes_arrow_selection() {
    let widget = TextInput::new("search")
        .initial_value("ab")
        .commit(TextCommit::OnEnterOrBlur);
    let _ = widget.cursor_position();
    let _ = widget.selected_text();
    let (mut harness, _) = Harness::new(widget, None).await;
    harness.focus();

    let left = harness.dispatch(
        key(
            Key::Named(NamedKey::ArrowLeft),
            None,
            ModifiersState::default(),
        ),
        None,
        None,
    );
    assert!(
        !left
            .param_assignments
            .iter()
            .any(|assignment| assignment.name == "search__value")
    );
    assert!(
        !left
            .commands
            .iter()
            .any(|command| matches!(command, RuntimeHostCommand::RequestWakeup { .. }))
    );
    assert_eq!(
        left.param_assignments
            .iter()
            .find(|assignment| assignment.name == "search__cursor")
            .map(|assignment| &assignment.value),
        Some(&ScalarValue::UInt64(Some(1)))
    );

    let typed = harness.dispatch(
        key(Key::Character('x'), Some("x"), ModifiersState::default()),
        None,
        None,
    );
    assert!(
        !typed
            .param_assignments
            .iter()
            .any(|assignment| assignment.name == "search__value")
    );
    let outside = [harness.frame_size[0] + 10.0, harness.frame_size[1] + 10.0];
    let blur = harness.dispatch(mouse_down(outside), None, Some(outside));
    assert_eq!(
        blur.param_assignments
            .iter()
            .find(|assignment| assignment.name == "search__value")
            .map(|assignment| &assignment.value),
        Some(&ScalarValue::Utf8(Some("axb".into())))
    );
}

#[tokio::test]
async fn multi_preedit_commit_is_one_undo_and_external_sync_is_latest_wins() {
    let widget = TextInput::new("search")
        .initial_value("A")
        .commit(TextCommit::OnEnterOrBlur);
    let (mut harness, _) = Harness::new(widget, None).await;
    harness.focus();

    for preedit in ["に", "日本"] {
        let outcome = harness.dispatch(
            SceneGraphEvent::Ime(ImeEvent::Preedit {
                text: preedit.into(),
                cursor: Some((preedit.len(), preedit.len())),
            }),
            None,
            None,
        );
        assert!(
            !outcome
                .param_assignments
                .iter()
                .any(|assignment| assignment.name == "search__value")
        );
    }

    harness.session.apply_param_patch(IndexMap::from([(
        "search__value".to_string(),
        ScalarValue::Utf8(Some("external-one".into())),
    )]));
    harness
        .session
        .evaluate(EvaluationRequest::new().at(harness.now + Duration::from_millis(10)))
        .await
        .unwrap();
    harness.session.apply_param_patch(IndexMap::from([(
        "search__value".to_string(),
        ScalarValue::Utf8(Some("external-two".into())),
    )]));
    harness
        .session
        .evaluate(EvaluationRequest::new().at(harness.now + Duration::from_millis(20)))
        .await
        .unwrap();

    let commit = harness.dispatch_at(
        SceneGraphEvent::Ime(ImeEvent::Commit("日本".into())),
        None,
        None,
        harness.now + Duration::from_millis(30),
    );
    let scene = harness
        .apply_and_evaluate(&commit, harness.now + Duration::from_millis(30))
        .await;
    assert_eq!(text_value(&scene), "external-two");

    // A fresh composition with no external replacement contributes one
    // committed edit regardless of its number of preedit updates.
    harness.dispatch_at(
        key(Key::Named(NamedKey::End), None, ModifiersState::default()),
        None,
        None,
        harness.now + Duration::from_millis(40),
    );
    for preedit in ["x", "xy", "xyz"] {
        harness.dispatch_at(
            SceneGraphEvent::Ime(ImeEvent::Preedit {
                text: preedit.into(),
                cursor: Some((preedit.len(), preedit.len())),
            }),
            None,
            None,
            harness.now + Duration::from_millis(50),
        );
    }
    let committed = harness.dispatch_at(
        SceneGraphEvent::Ime(ImeEvent::Commit("xyz".into())),
        None,
        None,
        harness.now + Duration::from_millis(60),
    );
    let committed_scene = harness
        .apply_and_evaluate(&committed, harness.now + Duration::from_millis(60))
        .await;
    assert_eq!(text_value(&committed_scene), "external-twoxyz");
    let undo = harness.dispatch_at(
        key(Key::Character('z'), None, command_modifier()),
        None,
        None,
        harness.now + Duration::from_millis(70),
    );
    let undo_scene = harness
        .apply_and_evaluate(&undo, harness.now + Duration::from_millis(70))
        .await;
    assert_eq!(text_value(&undo_scene), "external-two");
}

#[tokio::test]
async fn css_geometry_drives_measurement_scene_hit_and_ime_anchor() {
    let mut theme = Theme::light();
    theme
        .append_css(
            r#"
            text-input#search { height: 44px; min-width: 180px; }
            text-input#search::part(box) {
                input-inline-inset: 17px;
                corner-radius: 9px;
                stroke-width: 3px;
            }
            text-input#search::part(text) { font-size: 18px; font-family: "Lato"; }
            text-input#search::part(placeholder) { fill: #D55E00; }
            text-input#search::part(selection) { fill: #009E73; opacity: 0.5; }
            text-input#search::part(caret) { input-caret-width: 4px; }
            text-input#search::part(focus-ring) {
                focus-ring-width: 5px;
                stroke-width: 5px;
                focus-gap: 4px;
            }
            "#,
        )
        .unwrap();
    let initial = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let widget = TextInput::new("search")
        .placeholder("Search")
        .initial_value(initial);
    let _cursor = widget.cursor_position();
    let (mut harness, evaluated) = Harness::new(widget, Some(theme)).await;
    assert_eq!(harness.frame_size[1], 44.0);
    assert!(harness.frame_size[0] >= 180.0);
    let group = find_group(&evaluated.scene_graph.marks, "search").unwrap();
    let box_mark = find_rect(&group.marks, "box").unwrap();
    assert_eq!(box_mark.corner_radius.as_vec(1, None), [9.0]);
    assert_eq!(box_mark.stroke_width.as_vec(1, None), [3.0]);
    let text = find_text(&group.marks, "text").unwrap();
    assert!(text.x.as_vec(1, None)[0] < 17.0);
    assert_eq!(text.font_size.as_vec(1, None), [18.0]);
    let placeholder = find_text(&group.marks, "placeholder").unwrap();
    assert_eq!(
        placeholder.color.as_vec(1, None),
        [avenger_color::ColorOrGradient::Color(
            avenger_color::parse_color_string("#D55E00").unwrap()
        )]
    );

    let point = [17.0, harness.frame_size[1] * 0.5];
    let focus = harness.dispatch(mouse_down(point), Some("box"), Some(point));
    assert!(focus.param_assignments.iter().any(|assignment| {
        assignment.name == "search__cursor"
            && matches!(&assignment.value, ScalarValue::UInt64(Some(cursor)) if *cursor > 0 && *cursor < initial.len() as u64)
    }));
    assert!(focus.commands.iter().any(|command| matches!(
        command,
        RuntimeHostCommand::SetImeCursorArea { rect: Some(rect) }
            if rect.x() >= 17.0 && rect.x() <= harness.frame_size[0] && rect.width() == 4.0
    )));
    let focused = harness
        .apply_and_evaluate(&focus, harness.now + Duration::from_millis(1))
        .await;
    let group = find_group(&focused.scene_graph.marks, "search").unwrap();
    let ring = find_rect(&group.marks, "focus-ring").unwrap();
    assert_eq!(ring.x.as_vec(1, None), [-4.0]);
    assert_eq!(ring.stroke_width.as_vec(1, None), [5.0]);
    let caret = find_rule(&group.marks, "caret").unwrap();
    assert_eq!(caret.stroke_width.as_vec(1, None), [4.0]);
    let scrolled_text = find_text(&group.marks, "text").unwrap();
    assert!(scrolled_text.x.as_vec(1, None)[0] < 17.0);

    let select_all = harness.dispatch(
        key(Key::Character('a'), None, command_modifier()),
        None,
        None,
    );
    let selected = harness
        .apply_and_evaluate(&select_all, harness.now + Duration::from_millis(2))
        .await;
    let group = find_group(&selected.scene_graph.marks, "search").unwrap();
    let selection = find_rect(&group.marks, "selection").unwrap();
    let expected = avenger_color::parse_color_string("#009E73").unwrap();
    let actual = selection.fill.as_vec(selection.len as usize, None);
    assert!(!actual.is_empty());
    assert!(actual.iter().all(|fill| matches!(
        fill,
        avenger_color::ColorOrGradient::Color(color)
            if color[..3] == expected[..3] && (color[3] - 0.5).abs() < f32::EPSILON
    )));

    // Releasing the mouse ends drag selection; later movement is ignored.
    let up = harness.dispatch(
        mouse_up([harness.frame_size[0] - 8.0, harness.frame_size[1] * 0.5]),
        Some("box"),
        Some([harness.frame_size[0] - 8.0, harness.frame_size[1] * 0.5]),
    );
    assert!(up.consume);
}

#[tokio::test]
async fn canceled_composition_clipboard_and_unmount_have_exact_host_effects() {
    let widget = TextInput::new("search").initial_value("A").debounce(150);
    let (mut harness, _) = Harness::new(widget, None).await;
    let focus = harness.focus();
    assert!(matches!(
        focus.commands.as_slice(),
        [
            RuntimeHostCommand::SetImeAllowed { allowed: true },
            RuntimeHostCommand::SetImeCursorArea { rect: Some(_) }
        ]
    ));

    harness.dispatch(
        SceneGraphEvent::Ime(ImeEvent::Preedit {
            text: "temporary".into(),
            cursor: Some((9, 9)),
        }),
        None,
        None,
    );
    let canceled = harness.dispatch(SceneGraphEvent::Ime(ImeEvent::Disabled), None, None);
    let canceled_scene = harness
        .apply_and_evaluate(&canceled, harness.now + Duration::from_millis(1))
        .await;
    assert_eq!(text_value(&canceled_scene), "A");
    let undo_empty = harness.dispatch(
        key(Key::Character('z'), None, command_modifier()),
        None,
        None,
    );
    assert!(!undo_empty.scene_dirty);

    harness.dispatch(
        key(Key::Character('a'), None, command_modifier()),
        None,
        None,
    );
    let copy = harness.dispatch(SceneGraphEvent::Clipboard(ClipboardEvent::Copy), None, None);
    assert_eq!(
        copy.commands,
        [RuntimeHostCommand::WriteClipboard {
            text: "A".to_string()
        }]
    );

    let typed = harness.dispatch(
        SceneGraphEvent::Clipboard(ClipboardEvent::Paste("B\n\u{0}C".into())),
        None,
        None,
    );
    assert!(
        typed
            .commands
            .iter()
            .any(|command| matches!(command, RuntimeHostCommand::RequestWakeup { .. }))
    );
    let pasted = harness
        .apply_and_evaluate(&typed, harness.now + Duration::from_millis(2))
        .await;
    assert_eq!(text_value(&pasted), "BC");

    let Harness {
        session,
        resources,
        plot_id,
        now,
        ..
    } = harness;
    drop(session);
    let commands = resources
        .evict_plot(plot_id, now + Duration::from_secs(1))
        .unwrap();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, RuntimeHostCommand::CancelWakeup { .. }))
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, RuntimeHostCommand::RequestWakeup { .. }))
    );
}
