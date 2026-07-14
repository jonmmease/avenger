use avenger_chart::prelude::{
    AvengerChartError, Cartesian, Chart, ChartEventType, ChartWidgetPlacementExt, ChromePosition,
    CompiledParamSpec, GridConcat, HConcat, NativeWidget, NativeWidgetMeasureSpec,
    NativeWidgetStateSpec, Param, Theme, WidgetCell, WidgetMeasureSpec,
};
use avenger_chart_core::CompiledWidget;
use avenger_chart_widgets::{Button, ButtonVariant};
use avenger_color::ColorOrGradient;
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
    rect::SceneRectMark,
};
use datafusion::{common::ScalarValue, prelude::SessionContext};

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
        .frames
        .values()
        .find(|frame| frame.widget_id == "clear")
        .unwrap();
    assert_eq!((frame.bounds.width, frame.bounds.height), (width, height));
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
        .frames
        .values()
        .find(|frame| frame.widget_id == "clear")
        .unwrap();

    compiled
        .append_css("button#clear::part(box) { button-min-width: 180px; }")
        .unwrap();
    let after = compiled.evaluate(&ctx, None).await.unwrap();
    let after_frame = after
        .widget_frames
        .frames
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
async fn native_widget_cell_registers_state_and_reports_unavailable_runtime() {
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
        Err(AvengerChartError::NativeWidgetRuntimeUnavailable { widget_id, kind })
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
        .frames
        .values()
        .find(|frame| frame.widget_id == "first")
        .unwrap();
    let second = evaluated
        .widget_frames
        .frames
        .values()
        .find(|frame| frame.widget_id == "second")
        .unwrap();
    assert!(second.bounds.x > first.bounds.x + first.bounds.width);
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
        .frames
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
