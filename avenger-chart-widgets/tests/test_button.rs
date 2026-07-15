use avenger_chart::prelude::{
    AvengerChartError, Cartesian, Chart, ChartAction, ChartEventType, ChartParamChangeBinding,
    ChartWidgetPlacementExt, ChromePosition, CompiledParamSpec, EvaluationRequest, GridConcat,
    HConcat, NativeWidget, NativeWidgetCtx, NativeWidgetHostServices, NativeWidgetHostTransform,
    NativeWidgetInstanceKey, NativeWidgetInstanceStore, NativeWidgetMeasureSpec,
    NativeWidgetNamespace, NativeWidgetPlotId, NativeWidgetStateSpec, Param, PixelFrame, Theme,
    TrackSizing, WidgetCell, WidgetFrame, WidgetFrameAssignments, WidgetMeasureSpec,
};
use avenger_chart_core::CompiledWidget;
use avenger_chart_widgets::{Button, ButtonVariant};
use avenger_color::ColorOrGradient;
use avenger_eventstream::runtime::{LogicalRect, RuntimeHostCommand};
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
    rect::SceneRectMark,
};
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;
use std::sync::{Arc, Mutex};

struct TestNativeWidget;

impl NativeWidget for TestNativeWidget {
    fn id(&self) -> &str {
        "native"
    }

    fn kind(&self) -> &'static str {
        "test-native"
    }

    fn schema_version(&self) -> u32 {
        1
    }

    fn payload(&self) -> serde_json::Value {
        serde_json::json!({"label": "Native"})
    }

    fn measure(&self) -> NativeWidgetMeasureSpec {
        NativeWidgetMeasureSpec::Declarative(WidgetMeasureSpec::default())
    }

    fn state(&self) -> NativeWidgetStateSpec {
        NativeWidgetStateSpec::try_new(vec![CompiledParamSpec::shared(&Param::new(
            "native_value",
            false,
        ))])
        .unwrap()
    }
}

fn find_group<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneGroup> {
    for mark in marks {
        if let SceneMark::Group(group) = mark {
            if group.name == name {
                return Some(group);
            }
            if let Some(group) = find_group(&group.marks, name) {
                return Some(group);
            }
        }
    }
    None
}

#[tokio::test]
async fn button_widget_cell_round_trips_and_uses_intrinsic_size() {
    let ctx = SessionContext::new();
    let compiled = Chart::<HConcat>::new()
        .mark(WidgetCell::widget(Button::new("clear").label("Clear")).name("controls"))
        .compile(&ctx)
        .await
        .unwrap();
    let compiled: avenger_chart::plot::CompiledPlot =
        bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();

    assert_eq!(
        compiled.get_default_params().get("clear__activations"),
        Some(&ScalarValue::UInt64(Some(0)))
    );
    let click = compiled
        .event_bindings()
        .iter()
        .find(|binding| binding.event_type == ChartEventType::Click)
        .unwrap();
    assert_eq!(
        click.mark_ids(),
        &["controls.clear.box", "controls.clear.label"]
    );

    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    let controls = find_group(&evaluated.scene_graph.marks, "controls").unwrap();
    let clear = find_group(&controls.marks, "clear").unwrap();
    let box_mark = find_rect(&clear.marks, "box").unwrap();
    let width = box_mark.x2_vec()[0] - box_mark.x_vec()[0];
    let height = box_mark.y2_vec()[0] - box_mark.y_vec()[0];
    assert!(width > 0.0 && height > 0.0);
    assert_ne!(width, 400.0);
    assert_ne!(height, 300.0);
    let frame = evaluated
        .widget_frames
        .by_mark_path
        .values()
        .find(|frame| frame.widget_id == "clear")
        .unwrap();
    assert_eq!((frame.bounds.width, frame.bounds.height), (width, height));
}

#[tokio::test]
async fn nested_widget_cell_caret_composes_frame_and_host_offsets() {
    let ctx = SessionContext::new();
    let evaluated = Chart::<HConcat>::new()
        .mark(WidgetCell::widget(Button::new("clear").label("Clear")).name("controls"))
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();
    let frame = evaluated.widget_frames.by_widget_id.get("clear").unwrap();
    let namespace = NativeWidgetNamespace::new(
        Default::default(),
        NativeWidgetPlotId::from_member_path("root/controls"),
    );
    let store = avenger_chart::prelude::InMemoryNativeWidgetInstanceStore::new();
    let slot = store.slot(NativeWidgetInstanceKey::new(namespace, "clear"));
    let services = NativeWidgetHostServices::new();
    let sink = Arc::new(Mutex::new(Vec::new()));
    let transform = NativeWidgetHostTransform::from_offsets([
        [frame.bounds.x, frame.bounds.y],
        [30.0, 40.0],
        [-2.0, 3.0],
    ])
    .unwrap();
    let widget_ctx = NativeWidgetCtx::attach(slot, services, sink.clone(), transform);
    widget_ctx.focus(
        Some(LogicalRect::new(4.0, 5.0, 1.0, 12.0).unwrap()),
        "selection",
    );
    assert_eq!(
        sink.lock().unwrap().as_slice(),
        [
            RuntimeHostCommand::SetImeAllowed { allowed: true },
            RuntimeHostCommand::SetImeCursorArea {
                rect: LogicalRect::new(frame.bounds.x + 32.0, frame.bounds.y + 48.0, 1.0, 12.0,),
            },
        ]
    );
}

#[tokio::test]
async fn widget_cell_css_change_remeasures_without_recompilation() {
    let ctx = SessionContext::new();
    let mut compiled = Chart::<HConcat>::new()
        .mark(WidgetCell::widget(Button::new("clear").label("Clear")).name("controls"))
        .compile(&ctx)
        .await
        .unwrap();
    let before = compiled.evaluate(&ctx, None).await.unwrap();
    let before_frame = before
        .widget_frames
        .by_mark_path
        .values()
        .find(|frame| frame.widget_id == "clear")
        .unwrap();

    compiled
        .append_css("button#clear::part(box) { button-min-width: 180px; }")
        .unwrap();
    let after = compiled.evaluate(&ctx, None).await.unwrap();
    let after_frame = after
        .widget_frames
        .by_mark_path
        .values()
        .find(|frame| frame.widget_id == "clear")
        .unwrap();
    assert!(after_frame.bounds.width > before_frame.bounds.width);
    assert_eq!(after_frame.bounds.width, 180.0);
    let hit_point = [
        after_frame.bounds.x + 175.0,
        after_frame.bounds.y + after_frame.bounds.height / 2.0,
    ];

    let after_rtree = after.rtree.unwrap();
    assert!(
        after_rtree
            .locate_all_at_point(&hit_point)
            .any(|geometry| geometry.mark_instance.name == "box")
    );
}

#[tokio::test]
async fn native_widget_cell_registers_state_and_reports_unknown_kind_without_factory() {
    let ctx = SessionContext::new();
    let compiled = Chart::<HConcat>::new()
        .mark(WidgetCell::native_widget(TestNativeWidget).name("native_controls"))
        .compile(&ctx)
        .await
        .unwrap();
    let compiled: avenger_chart::plot::CompiledPlot =
        bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();
    assert_eq!(
        compiled.get_default_params().get("native_value"),
        Some(&ScalarValue::Boolean(Some(false)))
    );
    assert!(matches!(
        compiled.evaluate(&ctx, None).await,
        Err(AvengerChartError::UnknownNativeWidgetKind { widget_id, kind })
            if widget_id == "native" && kind == "test-native"
    ));
}

#[tokio::test]
async fn widget_cells_use_explicit_grid_placement() {
    let ctx = SessionContext::new();
    let evaluated = Chart::<GridConcat>::new()
        .configure_coord(|grid| grid.rows(1).columns(2))
        .mark(
            WidgetCell::widget(Button::new("first").label("First"))
                .name("left_controls")
                .at(0, 0),
        )
        .mark(
            WidgetCell::widget(Button::new("second").label("Second"))
                .name("right_controls")
                .at(0, 1),
        )
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();
    let first = evaluated
        .widget_frames
        .by_mark_path
        .values()
        .find(|frame| frame.widget_id == "first")
        .unwrap();
    let second = evaluated
        .widget_frames
        .by_mark_path
        .values()
        .find(|frame| frame.widget_id == "second")
        .unwrap();
    assert!(second.bounds.x > first.bounds.x + first.bounds.width);
}

#[tokio::test]
async fn content_widget_cell_keeps_preferred_size_in_flex_track() {
    let ctx = SessionContext::new();
    let auto = Chart::<HConcat>::new()
        .plot_size(300.0, 80.0)
        .mark(WidgetCell::widget(Button::new("auto").label("Button")).name("auto_cell"))
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();
    let flex = Chart::<HConcat>::new()
        .configure_coord(|coord| coord.widths([TrackSizing::Flex(1.0)]))
        .plot_size(300.0, 80.0)
        .mark(WidgetCell::widget(Button::new("flex").label("Button")).name("flex_cell"))
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();
    let frame_width = |evaluated: &avenger_chart::render::EvaluatedPlot, id: &str| {
        evaluated
            .widget_frames
            .by_mark_path
            .values()
            .find(|frame| frame.widget_id == id)
            .unwrap()
            .bounds
            .width
    };
    let auto_width = frame_width(&auto, "auto");
    assert_eq!(frame_width(&flex, "flex"), auto_width);
    assert!(auto_width < 300.0);
}

fn explicit_frames(first: WidgetFrame, second: WidgetFrame) -> WidgetFrameAssignments {
    WidgetFrameAssignments::try_from_iter([("first", first), ("second", second)]).unwrap()
}

#[tokio::test]
async fn explicit_widget_frames_validate_and_reassign_without_recompile() {
    assert!(WidgetFrame::try_new(f32::NAN, 0.0, 10.0, 10.0).is_err());
    assert!(WidgetFrame::try_new(0.0, 0.0, -1.0, 10.0).is_err());
    let unit = WidgetFrame::try_new(0.0, 0.0, 10.0, 10.0).unwrap();
    assert!(WidgetFrameAssignments::try_from_iter([("first", unit), ("first", unit)]).is_err());

    let ctx = Arc::new(SessionContext::new());
    let compiled = Chart::<PixelFrame>::new()
        .canvas_size(360.0, 180.0)
        .host_widget(
            Button::new("first")
                .label("First")
                .variant(ButtonVariant::Accent),
        )
        .host_widget(Button::new("second").label("Second"))
        .widget(
            Button::new("guide")
                .label("Guide")
                .position(ChromePosition::Left),
        )
        .compile(ctx.as_ref())
        .await
        .unwrap();
    let mut session = Arc::new(compiled).instantiate(ctx);

    let error = session
        .evaluate(EvaluationRequest::new())
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("Missing explicit widget frame assignments"));

    let unknown = WidgetFrameAssignments::try_from_iter([("unknown", unit)]).unwrap();
    let error = session
        .evaluate(EvaluationRequest::new().widget_frames(unknown))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("unknown explicit widget 'unknown'"));

    let guide = WidgetFrameAssignments::try_from_iter([("guide", unit)]).unwrap();
    let error = session
        .evaluate(EvaluationRequest::new().widget_frames(guide))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("guide-positioned widget"));

    let initial = session
        .evaluate(EvaluationRequest::new().widget_frames(explicit_frames(
            WidgetFrame::try_new(20.0, 30.0, 100.0, 36.0).unwrap(),
            WidgetFrame::try_new(150.0, 30.0, 120.0, 36.0).unwrap(),
        )))
        .await
        .unwrap();
    assert_eq!(
        initial.widget_frames.by_widget_id["first"].bounds,
        avenger_chart::layout::LayoutBounds {
            x: 20.0,
            y: 30.0,
            width: 100.0,
            height: 36.0,
        }
    );
    assert_eq!(initial.widget_frames.by_widget_id["second"].bounds.x, 150.0);
    let initial_first_box = find_rect(
        &find_group(&initial.scene_graph.marks, "first")
            .unwrap()
            .marks,
        "box",
    )
    .unwrap();
    assert_eq!(initial_first_box.x2_vec(), vec![100.0]);
    assert!(matches!(
        initial_first_box.fill.as_vec(1, None)[0],
        ColorOrGradient::Color(color)
            if color == avenger_color::parse_color_string("#0072B2").unwrap()
    ));

    let moved = session
        .evaluate(EvaluationRequest::new().widget_frames(explicit_frames(
            WidgetFrame::try_new(45.0, 70.0, 100.0, 36.0).unwrap(),
            WidgetFrame::try_new(175.0, 70.0, 120.0, 36.0).unwrap(),
        )))
        .await
        .unwrap();
    assert_eq!(moved.widget_frames.by_widget_id["first"].bounds.x, 45.0);
    assert_eq!(moved.widget_frames.by_widget_id["first"].bounds.y, 70.0);
    let moved_first_box = find_rect(
        &find_group(&moved.scene_graph.marks, "first").unwrap().marks,
        "box",
    )
    .unwrap();
    assert_eq!(moved_first_box.x2_vec(), initial_first_box.x2_vec());
    assert!(
        moved
            .rtree
            .as_ref()
            .unwrap()
            .locate_all_at_point(&[140.0, 88.0])
            .any(|geometry| geometry.mark_instance.name == "box")
    );

    let preview_moved = session
        .evaluate(
            EvaluationRequest::new()
                .widget_frames(explicit_frames(
                    WidgetFrame::try_new(55.0, 80.0, 100.0, 36.0).unwrap(),
                    WidgetFrame::try_new(185.0, 80.0, 120.0, 36.0).unwrap(),
                ))
                .preview(),
        )
        .await
        .unwrap();
    assert_eq!(
        preview_moved.widget_frames.by_widget_id["first"].bounds.x,
        55.0
    );
    let preview_first_box = find_rect(
        &find_group(&preview_moved.scene_graph.marks, "first")
            .unwrap()
            .marks,
        "box",
    )
    .unwrap();
    assert_eq!(preview_first_box.x2_vec(), initial_first_box.x2_vec());

    let resized = session
        .evaluate(EvaluationRequest::new().widget_frames(explicit_frames(
            WidgetFrame::try_new(45.0, 70.0, 180.0, 44.0).unwrap(),
            WidgetFrame::try_new(250.0, 70.0, 80.0, 44.0).unwrap(),
        )))
        .await
        .unwrap();
    assert_eq!(
        resized.widget_frames.by_widget_id["first"].bounds.width,
        180.0
    );
    let resized_first_box = find_rect(
        &find_group(&resized.scene_graph.marks, "first")
            .unwrap()
            .marks,
        "box",
    )
    .unwrap();
    assert_eq!(resized_first_box.x2_vec(), vec![180.0]);
}

fn find_rect<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneRectMark> {
    for mark in marks {
        match mark {
            SceneMark::Rect(rect) if rect.name == name => return Some(rect),
            SceneMark::Group(group) => {
                if let Some(rect) = find_rect(&group.marks, name) {
                    return Some(rect);
                }
            }
            _ => {}
        }
    }
    None
}

#[tokio::test]
async fn button_round_trips_counter_contract_and_accent_presentation() {
    let ctx = SessionContext::new();
    let compiled = Chart::<Cartesian>::new()
        .widget(
            Button::new("clear")
                .label("Clear")
                .variant(ButtonVariant::Accent)
                .position(ChromePosition::Left),
        )
        .compile(&ctx)
        .await
        .unwrap();
    let compiled: avenger_chart::plot::CompiledPlot =
        bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();

    assert_eq!(
        compiled.get_default_params().get("clear__activations"),
        Some(&ScalarValue::UInt64(Some(0)))
    );
    assert_eq!(compiled.cursor_params(), &["clear__cursor"]);
    assert!(compiled.param_change_bindings().is_empty());
    for event_type in [
        ChartEventType::Click,
        ChartEventType::MarkMouseEnter,
        ChartEventType::MarkMouseLeave,
    ] {
        let binding = compiled
            .event_bindings()
            .iter()
            .find(|binding| binding.event_type == event_type)
            .unwrap();
        assert_eq!(binding.mark_ids(), &["clear.box", "clear.label"]);
    }
    let CompiledWidget::Composed(widget) = &compiled.widgets()[0].widget else {
        panic!("expected composed button")
    };
    assert_eq!(widget.presentation.variant.as_deref(), Some("accent"));

    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    let group = find_group(&evaluated.scene_graph.marks, "clear").unwrap();
    let box_mark = find_rect(&group.marks, "box").unwrap();
    assert!(matches!(
        box_mark.fill.as_vec(1, None)[0],
        ColorOrGradient::Color(color)
            if color == avenger_color::parse_color_string("#0072B2").unwrap()
    ));
    assert!(!find_rect(&group.marks, "focus-ring").unwrap().interactive);
}

#[tokio::test]
async fn button_action_matches_explicit_binding_and_round_trips() {
    let ctx = SessionContext::new();
    let target = Param::new("target", 7_i64);
    let button = Button::new("clear").label("Clear");
    let activation = button.activation_param();
    let action = ChartAction::new().reset_param(&target).exact();

    let sugar = Chart::<Cartesian>::new()
        .param(target.clone())
        .widget(
            button
                .clone()
                .action(action.clone())
                .position(ChromePosition::Left),
        )
        .compile(&ctx)
        .await
        .expect("compile button action sugar");
    let explicit = Chart::<Cartesian>::new()
        .param(target)
        .param_change_binding(ChartParamChangeBinding::on(&activation).then(action))
        .widget(button.position(ChromePosition::Left))
        .compile(&ctx)
        .await
        .expect("compile explicit button action binding");

    assert_eq!(
        bincode::serialize(&sugar).expect("serialize sugar artifact"),
        bincode::serialize(&explicit).expect("serialize explicit artifact")
    );
    let restored: avenger_chart::plot::CompiledPlot = bincode::deserialize(
        &bincode::serialize(&sugar).expect("serialize button action artifact"),
    )
    .expect("deserialize button action artifact");
    assert_eq!(restored.param_change_bindings().len(), 1);
    assert_eq!(
        restored.param_change_bindings()[0].source_param_name,
        "clear__activations"
    );

    let inactive = restored
        .evaluate(&ctx, None)
        .await
        .expect("evaluate inactive button");
    let inactive_marks = inactive.scene_graph.marks;
    let mut params = IndexMap::new();
    params.insert(
        "clear__activations".to_string(),
        ScalarValue::UInt64(Some(1)),
    );
    let mut session = Arc::new(restored).instantiate(Arc::new(ctx));
    let active = session
        .evaluate(EvaluationRequest::new().params(params))
        .await
        .expect("evaluate activated button");
    assert_eq!(
        inactive_marks, active.scene_graph.marks,
        "the monotonic activation count must not create a latched visual state"
    );
}

#[tokio::test]
async fn button_actions_keep_activation_sources_and_targets_disjoint() {
    let ctx = SessionContext::new();
    let first_target = Param::new("first_target", false);
    let second_target = Param::new("second_target", false);
    let compiled = Chart::<Cartesian>::new()
        .param(first_target.clone())
        .param(second_target.clone())
        .widget(
            Button::new("first")
                .label("First")
                .action(ChartAction::new().set_param(&first_target, true))
                .position(ChromePosition::Left),
        )
        .widget(
            Button::new("second")
                .label("Second")
                .action(ChartAction::new().set_param(&second_target, true))
                .position(ChromePosition::Left),
        )
        .compile(&ctx)
        .await
        .expect("compile disjoint button actions");

    let bindings = compiled.param_change_bindings();
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0].source_param_name, "first__activations");
    assert_eq!(bindings[0].action.assignments[0].param_name, "first_target");
    assert_eq!(bindings[1].source_param_name, "second__activations");
    assert_eq!(
        bindings[1].action.assignments[0].param_name,
        "second_target"
    );
}

#[tokio::test]
async fn button_css_geometry_drives_minimum_frame_and_centered_parts() {
    let ctx = SessionContext::new();
    let mut theme = Theme::light();
    theme
        .append_css(
            "button { height: 38px; } \
             button::part(box) { button-min-width: 90px; button-inline-padding: 20px; } \
             button::part(focus-ring) { focus-gap: 3px; }",
        )
        .unwrap();
    let evaluated = Chart::<Cartesian>::new()
        .theme(theme)
        .widget(Button::new("go").label("Go").position(ChromePosition::Left))
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let group = find_group(&evaluated.scene_graph.marks, "go").unwrap();
    assert_eq!(group.clip, Clip::None);
    let frame = evaluated
        .widget_frames
        .by_mark_path
        .values()
        .find(|frame| frame.widget_id == "go")
        .unwrap();
    assert_eq!((frame.bounds.width, frame.bounds.height), (90.0, 38.0));
    let box_mark = find_rect(&group.marks, "box").unwrap();
    assert_eq!(box_mark.x_vec(), vec![0.0]);
    assert_eq!(box_mark.x2_vec(), vec![90.0]);
    assert_eq!(box_mark.y_vec(), vec![0.0]);
    assert_eq!(box_mark.y2_vec(), vec![38.0]);

    let label = group
        .marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Text(text) if text.name == "label" => Some(text),
            _ => None,
        })
        .unwrap();
    assert_eq!(label.x.as_vec(1, None), vec![45.0]);
    assert_eq!(label.y.as_vec(1, None), vec![19.0]);

    let focus = find_rect(&group.marks, "focus-ring").unwrap();
    assert_eq!(focus.x_vec(), vec![-3.0]);
    assert_eq!(focus.x2_vec(), vec![93.0]);
    assert_eq!(focus.y_vec(), vec![-3.0]);
    assert_eq!(focus.y2_vec(), vec![41.0]);
}

#[tokio::test]
async fn button_rejects_missing_label_and_non_unsigned_counter() {
    let ctx = SessionContext::new();
    assert!(
        Chart::<Cartesian>::new()
            .widget(Button::new("empty").position(ChromePosition::Left))
            .compile(&ctx)
            .await
            .is_err()
    );
    assert!(
        Chart::<Cartesian>::new()
            .widget(
                Button::new("bad")
                    .label("Bad")
                    .with_activation_param(avenger_chart::prelude::Param::new("bad_count", 0_i64))
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .is_err()
    );
}
