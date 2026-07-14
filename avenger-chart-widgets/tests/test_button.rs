use avenger_chart::prelude::{
    Cartesian, Chart, ChartEventType, ChartWidgetPlacementExt, ChromePosition, Theme,
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
