use avenger_common::time::Instant;
use avenger_eventstream::{
    scene::*,
    window::{Key, MouseButton, NamedKey},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::{marks::rect::SceneRectMark, scene_graph::SceneGraph};
use avenger_widgets::prelude::*;

fn frame(runtime: &mut WidgetRuntime, specs: Vec<WidgetSpec>) -> SceneGraph {
    let engine = avenger_text::default_text_engine();
    let mut prepared = runtime
        .prepare(&specs, &WidgetTheme::light(), &engine)
        .unwrap();
    for (i, spec) in specs.iter().enumerate() {
        prepared
            .place(
                spec.id().clone(),
                Rect::new(10.0, 10.0 + i as f32 * 50.0, 180.0, 36.0),
                None,
            )
            .unwrap();
    }
    let frame = prepared.finish().unwrap();
    let scene = SceneGraph {
        marks: vec![frame.scene.clone().into()],
        width: 240.0,
        height: 300.0,
        origin: [0.0; 2],
    };
    runtime.install(frame).unwrap();
    scene
}
fn key(key: NamedKey, repeat: bool) -> SceneGraphEvent {
    SceneGraphEvent::KeyPress(SceneKeyPressEvent {
        key: Key::Named(key),
        repeat,
        position: None,
        text: None,
        mark_instance: None,
        modifiers: ModifiersState::default(),
    })
}
fn release(key: NamedKey) -> SceneGraphEvent {
    SceneGraphEvent::KeyRelease(SceneKeyReleaseEvent {
        key: Key::Named(key),
        position: None,
        mark_instance: None,
        modifiers: ModifiersState::default(),
    })
}
fn mouse(down: bool, p: [f32; 2]) -> SceneGraphEvent {
    if down {
        SceneGraphEvent::MouseDown(SceneMouseDownEvent {
            position: p,
            button: MouseButton::Left,
            mark_instance: None,
            modifiers: ModifiersState::default(),
        })
    } else {
        SceneGraphEvent::MouseUp(SceneMouseUpEvent {
            position: p,
            button: MouseButton::Left,
            mark_instance: None,
            modifiers: ModifiersState::default(),
        })
    }
}
fn handle(r: &mut WidgetRuntime, scene: &SceneGraph, e: SceneGraphEvent) -> WidgetUpdate {
    r.handle(
        &e,
        &SceneGraphRTree::from_scene_graph(scene),
        Instant::now(),
    )
    .unwrap()
}
fn actions(u: &WidgetUpdate) -> Vec<&WidgetAction> {
    u.events
        .iter()
        .filter(|e| !matches!(e.action, WidgetAction::FocusChanged { .. }))
        .map(|e| &e.action)
        .collect()
}

#[test]
fn keyboard_before_pointer_uses_focus_order_and_ignores_activation_repeat() {
    let mut r = WidgetRuntime::new();
    let scene = frame(
        &mut r,
        vec![
            Button::new("a", "Apply").into(),
            Checkbox::new("off", "Disabled", false)
                .enabled(false)
                .into(),
            Checkbox::new("c", "Grid", false).into(),
        ],
    );
    assert!(
        handle(&mut r, &scene, key(NamedKey::Tab, false))
            .status
            .consume
    );
    assert_eq!(r.focused(), Some(&WidgetTarget::new("a")));
    assert_eq!(
        actions(&handle(&mut r, &scene, key(NamedKey::Enter, false))),
        vec![&WidgetAction::Activated]
    );
    assert!(actions(&handle(&mut r, &scene, key(NamedKey::Enter, true))).is_empty());
    handle(&mut r, &scene, key(NamedKey::Tab, false));
    assert_eq!(r.focused(), Some(&WidgetTarget::new("c")));
    handle(&mut r, &scene, key(NamedKey::Space, false));
    handle(&mut r, &scene, key(NamedKey::Space, true));
    assert_eq!(
        actions(&handle(&mut r, &scene, release(NamedKey::Space))),
        vec![&WidgetAction::CheckedChanged { value: true }]
    );
    assert!(actions(&handle(&mut r, &scene, release(NamedKey::Space))).is_empty());
    assert!(
        !handle(&mut r, &scene, key(NamedKey::Tab, false))
            .status
            .consume
    );
    assert!(r.focused().is_none());
}
#[test]
fn pointer_ownership_labels_reentry_and_overlays_match_scene_picking() {
    let mut r = WidgetRuntime::new();
    let mut scene = frame(
        &mut r,
        vec![Checkbox::new("c", "Clickable full label", false).into()],
    );
    assert!(
        handle(&mut r, &scene, mouse(true, [175.0, 25.0]))
            .status
            .suppress_click
    );
    let out = handle(&mut r, &scene, mouse(false, [220.0, 25.0]));
    assert!(actions(&out).is_empty());
    handle(&mut r, &scene, mouse(true, [175.0, 25.0]));
    handle(
        &mut r,
        &scene,
        SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
            position: [220.0, 25.0],
            mark_instance: None,
            modifiers: ModifiersState::default(),
        }),
    );
    assert_eq!(
        actions(&handle(&mut r, &scene, mouse(false, [175.0, 25.0]))),
        vec![&WidgetAction::CheckedChanged { value: true }]
    );
    scene.marks.push(
        SceneRectMark {
            name: "overlay".into(),
            x: 100.0.into(),
            y: 10.0.into(),
            width: Some(100.0.into()),
            height: Some(40.0.into()),
            ..Default::default()
        }
        .into(),
    );
    assert!(
        !handle(&mut r, &scene, mouse(true, [175.0, 25.0]))
            .status
            .consume
    );
    assert!(actions(&handle(&mut r, &scene, mouse(false, [175.0, 25.0]))).is_empty());
}
#[test]
fn removal_and_focus_loss_retire_pending_gestures() {
    let mut r = WidgetRuntime::new();
    let mut scene = frame(&mut r, vec![Button::new("a", "Go").into()]);
    handle(&mut r, &scene, mouse(true, [30.0, 25.0]));
    frame(&mut r, vec![]);
    scene = frame(&mut r, vec![Button::new("a", "New").into()]);
    assert!(actions(&handle(&mut r, &scene, mouse(false, [30.0, 25.0]))).is_empty());
    handle(&mut r, &scene, key(NamedKey::Tab, false));
    handle(&mut r, &scene, key(NamedKey::Space, false));
    handle(&mut r, &scene, SceneGraphEvent::WindowFocused(false));
    assert!(r.focused().is_none());
    handle(&mut r, &scene, SceneGraphEvent::WindowFocused(true));
    assert_eq!(r.focused(), Some(&WidgetTarget::new("a")));
    assert!(actions(&handle(&mut r, &scene, release(NamedKey::Space))).is_empty());
}
#[test]
fn preparation_is_transactional_and_stale_frames_cannot_replace_live_state() {
    let mut r = WidgetRuntime::new();
    let scene = frame(&mut r, vec![Checkbox::new("c", "Grid", false).into()]);
    let specs: Vec<WidgetSpec> = vec![Checkbox::new("c", "Grid", true).into()];
    let engine = avenger_text::default_text_engine();
    let theme = WidgetTheme::light();
    assert!(
        r.prepare(&[specs[0].clone(), specs[0].clone()], &theme, &engine)
            .is_err()
    );
    assert!(
        r.prepare(&[Button::new("c", "Changed kind").into()], &theme, &engine)
            .is_err()
    );
    let mut candidate = r.prepare(&specs, &theme, &engine).unwrap();
    candidate
        .place("c", Rect::new(0.0, 0.0, 100.0, 30.0), None)
        .unwrap();
    let stale = candidate.finish().unwrap();
    handle(&mut r, &scene, key(NamedKey::Tab, false));
    assert!(matches!(r.install(stale), Err(WidgetError::StaleFrame)));
    let cloned = r.clone();
    assert_eq!(cloned.focused(), r.focused());
    frame(&mut r, vec![Checkbox::new("c", "Grid", false).into()]);
    assert_eq!(
        r.semantics()[0].value,
        avenger_widgets::SemanticValue::Checked(false)
    );
}
#[test]
fn clipped_rows_cannot_be_picked_or_focused_and_styles_do_not_resize_on_hover() {
    let mut r = WidgetRuntime::new();
    let engine = avenger_text::default_text_engine();
    let theme = WidgetTheme::light();
    let specs = vec![Checkbox::new("c", "Grid", false).into()];
    let mut p = r.prepare(&specs, &theme, &engine).unwrap();
    let metrics = p.metrics("c").unwrap();
    p.place(
        "c",
        Rect::new(10.0, 10.0, 180.0, 36.0),
        Some(Rect::new(10.0, 10.0, 20.0, 36.0)),
    )
    .unwrap();
    let f = p.finish().unwrap();
    let scene = SceneGraph {
        marks: vec![f.scene.clone().into()],
        width: 240.0,
        height: 100.0,
        origin: [0.0; 2],
    };
    r.install(f).unwrap();
    assert!(
        !handle(&mut r, &scene, mouse(true, [100.0, 25.0]))
            .status
            .consume
    );
    handle(
        &mut r,
        &scene,
        SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
            position: [20.0, 25.0],
            mark_instance: None,
            modifiers: ModifiersState::default(),
        }),
    );
    assert_eq!(
        r.prepare(&specs, &theme, &engine)
            .unwrap()
            .metrics("c")
            .unwrap(),
        metrics
    );
    let mut p = r.prepare(&specs, &theme, &engine).unwrap();
    p.place(
        "c",
        Rect::new(10.0, 10.0, 180.0, 36.0),
        Some(Rect::new(0.0, 0.0, 0.0, 0.0)),
    )
    .unwrap();
    r.install(p.finish().unwrap()).unwrap();
    assert!(
        r.request_focus(Some(WidgetTarget::new("c")), Instant::now())
            .is_err()
    );
}

fn group_frame(
    runtime: &mut WidgetRuntime,
    specs: &[WidgetSpec],
    width: f32,
) -> (SceneGraph, WidgetUpdate) {
    let mut p = runtime
        .prepare(
            specs,
            &WidgetTheme::light(),
            &avenger_text::default_text_engine(),
        )
        .unwrap();
    let mut y = 10.0;
    for spec in specs {
        let h = p.metrics(spec.id().clone()).unwrap().preferred.height;
        p.place(spec.id().clone(), Rect::new(10.0, y, width, h), None)
            .unwrap();
        y += h + 15.0;
    }
    let f = p.finish().unwrap();
    let scene = SceneGraph {
        marks: vec![f.scene.clone().into()],
        width: 350.0,
        height: y,
        origin: [0.0; 2],
    };
    let update = runtime.install(f).unwrap();
    (scene, update)
}
fn items() -> Vec<ChoiceItem> {
    vec![
        ChoiceItem::new("a", "Alpha"),
        ChoiceItem::new("b", "Beta").enabled(false),
        ChoiceItem::new("c", "Gamma"),
    ]
}
#[test]
fn groups_have_distinct_tab_models_and_stable_item_identity() {
    let mut r = WidgetRuntime::new();
    let checked = std::collections::BTreeSet::from(["b".into()]);
    let specs = vec![
        CheckboxGroup::new("checks", items(), checked.clone()).into(),
        RadioGroup::new("radio", items(), Some("b".into())).into(),
        Button::new("end", "End").into(),
    ];
    let (scene, _) = group_frame(&mut r, &specs, 250.0);
    handle(&mut r, &scene, key(NamedKey::Tab, false));
    assert_eq!(r.focused(), Some(&WidgetTarget::item("checks", "a")));
    handle(&mut r, &scene, key(NamedKey::Tab, false));
    assert_eq!(r.focused(), Some(&WidgetTarget::item("checks", "c")));
    handle(&mut r, &scene, key(NamedKey::Space, false));
    let u = handle(&mut r, &scene, release(NamedKey::Space));
    let expected = std::collections::BTreeSet::from(["b".into(), "c".into()]);
    assert_eq!(
        actions(&u),
        vec![&WidgetAction::CheckedItemsChanged {
            item: "c".into(),
            checked: expected.clone()
        }]
    );
    let reordered = vec![
        ChoiceItem::new("c", "Renamed Gamma"),
        ChoiceItem::new("b", "Beta").enabled(false),
        ChoiceItem::new("a", "Alpha"),
    ];
    let (scene, _) = group_frame(
        &mut r,
        &[
            CheckboxGroup::new("checks", reordered, expected).into(),
            specs[1].clone(),
            specs[2].clone(),
        ],
        250.0,
    );
    assert_eq!(r.focused(), Some(&WidgetTarget::item("checks", "c")));
    handle(&mut r, &scene, key(NamedKey::Tab, false));
    assert_eq!(r.focused(), Some(&WidgetTarget::item("checks", "a")));
    handle(&mut r, &scene, key(NamedKey::Tab, false));
    assert_eq!(r.focused(), Some(&WidgetTarget::item("radio", "a")));
    assert_eq!(
        actions(&handle(&mut r, &scene, key(NamedKey::ArrowRight, true))),
        vec![&WidgetAction::SelectionChanged { item: "c".into() }]
    );
    assert_eq!(
        actions(&handle(&mut r, &scene, key(NamedKey::ArrowRight, true))),
        vec![&WidgetAction::SelectionChanged { item: "a".into() }]
    );
    handle(&mut r, &scene, key(NamedKey::Tab, false));
    assert_eq!(r.focused(), Some(&WidgetTarget::new("end")));
    let semantics = r.semantics();
    assert_eq!(semantics.iter().filter(|s| s.parent.is_some()).count(), 6);
}
#[test]
fn group_validation_and_removed_item_release_do_not_alias() {
    let mut r = WidgetRuntime::new();
    let engine = avenger_text::default_text_engine();
    let theme = WidgetTheme::light();
    assert!(
        r.prepare(
            &[CheckboxGroup::new(
                "g",
                vec![ChoiceItem::new("a", "One"), ChoiceItem::new("a", "Two")],
                []
            )
            .into()],
            &theme,
            &engine
        )
        .is_err()
    );
    assert!(
        r.prepare(
            &[CheckboxGroup::new("g", items(), ["missing".into()]).into()],
            &theme,
            &engine
        )
        .is_err()
    );
    assert!(
        r.prepare(
            &[RadioGroup::new("g", items(), Some("missing".into())).into()],
            &theme,
            &engine
        )
        .is_err()
    );
    let (scene, _) = group_frame(
        &mut r,
        &[CheckboxGroup::new("g", items(), []).into()],
        250.0,
    );
    handle(&mut r, &scene, mouse(true, [150.0, 25.0]));
    group_frame(&mut r, &[CheckboxGroup::new("g", [], []).into()], 250.0);
    let (scene, _) = group_frame(
        &mut r,
        &[CheckboxGroup::new("g", items(), []).into()],
        250.0,
    );
    assert!(actions(&handle(&mut r, &scene, mouse(false, [150.0, 25.0]))).is_empty());
}
#[test]
fn slider_keyboard_short_endpoint_repeat_and_transaction_order() {
    let mut r = WidgetRuntime::new();
    let d = SliderDomain::stepped(0.0, 10.0, 3.0).unwrap();
    let (scene, _) = group_frame(&mut r, &[Slider::new("s", d, 0.0).into()], 200.0);
    handle(&mut r, &scene, key(NamedKey::Tab, false));
    for value in [3.0, 6.0, 9.0, 10.0] {
        assert_eq!(
            actions(&handle(&mut r, &scene, key(NamedKey::ArrowRight, true))),
            vec![&WidgetAction::SliderChanged { value }]
        );
    }
    assert!(actions(&handle(&mut r, &scene, key(NamedKey::ArrowRight, true))).is_empty());
    assert_eq!(
        actions(&handle(&mut r, &scene, release(NamedKey::ArrowRight))),
        vec![&WidgetAction::SliderCommitted { value: 10.0 }]
    );
    assert!(actions(&handle(&mut r, &scene, release(NamedKey::ArrowRight))).is_empty());
    handle(&mut r, &scene, key(NamedKey::ArrowLeft, false));
    assert_eq!(
        actions(&handle(&mut r, &scene, key(NamedKey::Home, false))),
        vec![
            &WidgetAction::SliderCommitted { value: 9.0 },
            &WidgetAction::SliderChanged { value: 0.0 }
        ]
    );
    assert_eq!(
        actions(&handle(&mut r, &scene, key(NamedKey::Escape, false))),
        vec![
            &WidgetAction::SliderChanged { value: 9.0 },
            &WidgetAction::SliderCancelled {
                value: 9.0,
                reason: SliderCancelReason::Escape
            }
        ]
    );
    assert!(actions(&handle(&mut r, &scene, release(NamedKey::Home))).is_empty());
}
#[test]
fn slider_drag_preserves_grab_offset_and_programmatic_values_cancel() {
    let mut r = WidgetRuntime::new();
    let d = SliderDomain::continuous(0.0, 100.0).unwrap();
    let (scene, _) = group_frame(&mut r, &[Slider::new("s", d, 50.0).into()], 200.0);
    assert!(actions(&handle(&mut r, &scene, mouse(true, [114.0, 25.0]))).is_empty());
    let moved = handle(
        &mut r,
        &scene,
        SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
            position: [132.2, 25.0],
            mark_instance: None,
            modifiers: ModifiersState::default(),
        }),
    );
    assert!(
        matches!(actions(&moved).as_slice(),[WidgetAction::SliderChanged{value}] if (value-60.0).abs()<0.0001)
    );
    let (_, update) = group_frame(&mut r, &[Slider::new("s", d, 20.0).into()], 200.0);
    assert_eq!(
        actions(&update),
        vec![&WidgetAction::SliderCancelled {
            value: 20.0,
            reason: SliderCancelReason::Replaced
        }]
    );
    assert!(actions(&handle(&mut r, &scene, mouse(false, [150.0, 25.0]))).is_empty());
    let (scene, _) = group_frame(&mut r, &[Slider::new("s", d, 20.0).into()], 200.0);
    handle(&mut r, &scene, mouse(true, [150.0, 25.0]));
    let u = handle(&mut r, &scene, SceneGraphEvent::PointerCaptureLost);
    assert!(matches!(
        actions(&u).as_slice(),
        [WidgetAction::SliderCancelled {
            reason: SliderCancelReason::CaptureLost,
            ..
        }]
    ));
    assert!(actions(&handle(&mut r, &scene, mouse(false, [150.0, 25.0]))).is_empty());
    let (tiny, _) = group_frame(&mut r, &[Slider::new("s", d, 20.0).into()], 8.0);
    assert!(actions(&handle(&mut r, &tiny, mouse(true, [14.0, 25.0]))).is_empty());
    handle(&mut r, &tiny, mouse(false, [14.0, 25.0]));
    assert_eq!(
        actions(&handle(&mut r, &tiny, key(NamedKey::ArrowRight, false))),
        vec![&WidgetAction::SliderChanged { value: 21.0 }]
    );
}

#[test]
fn paint_only_frames_keep_the_installed_picking_geometry() {
    let mut runtime = WidgetRuntime::new();
    let specs = vec![Checkbox::new("check", "Grid", false).into()];
    let (scene, first) = group_frame(&mut runtime, &specs, 240.0);
    assert!(first.status.rebuild_geometry);
    handle(&mut runtime, &scene, key(NamedKey::Tab, false));
    let (_, focused) = group_frame(&mut runtime, &specs, 240.0);
    assert!(!focused.status.rebuild_geometry);
    let (_, changed) = group_frame(
        &mut runtime,
        &[Checkbox::new("check", "Grid", true).into()],
        240.0,
    );
    assert!(!changed.status.rebuild_geometry);
    let (_, moved) = group_frame(&mut runtime, &specs, 260.0);
    assert!(moved.status.rebuild_geometry);
}

#[test]
fn unlabeled_groups_report_the_first_row_baseline_and_empty_groups_have_none() {
    let runtime = WidgetRuntime::new();
    let specs: Vec<WidgetSpec> = vec![
        Checkbox::new("single", "Grid", false).into(),
        CheckboxGroup::new("group", vec![ChoiceItem::new("grid", "Grid")], []).into(),
        RadioGroup::new("radio", vec![ChoiceItem::new("grid", "Grid")], None).into(),
        RadioGroup::new("empty", vec![], None).into(),
    ];
    let prepared = runtime
        .prepare(
            &specs,
            &WidgetTheme::light(),
            &avenger_text::default_text_engine(),
        )
        .unwrap();
    assert_eq!(
        prepared.metrics("single").unwrap().baseline,
        prepared.metrics("group").unwrap().baseline
    );
    assert_eq!(
        prepared.metrics("single").unwrap().baseline,
        prepared.metrics("radio").unwrap().baseline
    );
    assert_eq!(prepared.metrics("empty").unwrap().baseline, None);
}

#[test]
fn modified_keys_pass_through_but_owned_key_releases_finish_the_gesture() {
    let mut runtime = WidgetRuntime::new();
    let scene = frame(
        &mut runtime,
        vec![Checkbox::new("grid", "Grid", false).into()],
    );
    handle(&mut runtime, &scene, key(NamedKey::Tab, false));
    let policy = handle(&mut runtime, &scene, key(NamedKey::Space, false))
        .status
        .commands
        .into_iter()
        .find_map(|c| {
            if let avenger_eventstream::runtime::RuntimeHostCommand::SetKeyboardPolicy { policy } =
                c
            {
                policy
            } else {
                None
            }
        })
        .unwrap();
    let modifiers = ModifiersState {
        control: true,
        ..Default::default()
    };
    assert!(!policy.captures(Key::Named(NamedKey::Space), modifiers));
    assert!(!policy.captures(Key::Named(NamedKey::Tab), modifiers));
    let mut end = release(NamedKey::Space);
    if let SceneGraphEvent::KeyRelease(e) = &mut end {
        e.modifiers = modifiers;
    }
    assert!(matches!(
        actions(&handle(&mut runtime, &scene, end))[0],
        WidgetAction::CheckedChanged { value: true }
    ));
    let mut tab = key(NamedKey::Tab, false);
    if let SceneGraphEvent::KeyPress(e) = &mut tab {
        e.modifiers = modifiers;
    }
    assert!(!handle(&mut runtime, &scene, tab).status.consume);
    assert_eq!(runtime.focused(), Some(&WidgetTarget::new("grid")));
}

#[test]
fn control_strokes_fit_inside_allocations_including_slider_endpoints() {
    use avenger_scenegraph::{
        marks::mark::SceneMark,
        render_order::{SceneDisplayList, SceneDisplayMark},
    };
    let mut theme = WidgetTheme::light();
    theme.radio.border_width = 4.0;
    theme.checkbox.border_width = 4.0;
    theme.button.border_width = 4.0;
    theme.text_input.border_width = 4.0;
    let domain = SliderDomain::continuous(0.0, 1.0).unwrap();
    let controls: Vec<WidgetSpec> = vec![
        Button::new("button", "Apply").into(),
        Checkbox::new("checkbox", "Grid", false).into(),
        CheckboxGroup::new("checks", vec![ChoiceItem::new("grid", "Grid")], []).into(),
        RadioGroup::new(
            "radio",
            vec![ChoiceItem::new("ocean", "Ocean")],
            Some("ocean".into()),
        )
        .into(),
        Slider::new("min", domain, 0.0).into(),
        Slider::new("max", domain, 1.0).into(),
        TextInput::new("text", "Source").into(),
    ];
    for control in controls {
        let runtime = WidgetRuntime::new();
        let mut prepared = runtime
            .prepare(
                std::slice::from_ref(&control),
                &theme,
                &avenger_text::default_text_engine(),
            )
            .unwrap();
        let size = prepared.metrics(control.id().clone()).unwrap().preferred;
        let bounds = Rect::new(10.0, 10.0, size.width, size.height);
        prepared.place(control.id().clone(), bounds, None).unwrap();
        let scene = SceneGraph {
            width: 400.0,
            height: 100.0,
            origin: [0.0; 2],
            marks: vec![prepared.finish().unwrap().scene.into()],
        };
        let mut borders = 0;
        for item in SceneDisplayList::from_scene_graph(&scene).items {
            let SceneDisplayMark::Borrowed(SceneMark::Rect(mark)) = item.mark else {
                continue;
            };
            let half_stroke = mark.stroke_width.first().unwrap() / 2.0;
            if half_stroke == 0.0 {
                continue;
            }
            borders += 1;
            let x = mark.x.first().unwrap() + item.origin[0];
            let y = mark.y.first().unwrap() + item.origin[1];
            let width = mark.width.as_ref().unwrap().first().unwrap();
            let height = mark.height.as_ref().unwrap().first().unwrap();
            assert!(x - half_stroke >= bounds.x, "{} left border", control.id());
            assert!(y - half_stroke >= bounds.y, "{} top border", control.id());
            assert!(
                x + width + half_stroke <= bounds.x + bounds.width,
                "{} right border",
                control.id()
            );
            assert!(
                y + height + half_stroke <= bounds.y + bounds.height,
                "{} bottom border",
                control.id()
            );
        }
        assert_eq!(borders, 1, "{} border coverage", control.id());
    }
}
