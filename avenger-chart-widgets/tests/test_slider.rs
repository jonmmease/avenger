use avenger_chart::prelude::{
    Cartesian, Chart, ChartEventType, ChartWidgetPlacementExt, ChromePosition, GridConcat, HConcat,
    Theme, TrackSizing, WidgetCell,
};
use avenger_chart_widgets::Slider;
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
    rect::SceneRectMark,
    symbol::SceneSymbolMark,
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

async fn slider_cell_frame_width(
    track: Option<TrackSizing>,
) -> (f32, avenger_chart::render::EvaluatedPlot) {
    let ctx = SessionContext::new();
    let mut chart = Chart::<HConcat>::new().plot_size(300.0, 80.0);
    if let Some(track) = track {
        chart = chart.configure_coord(|coord| coord.widths([track]));
    }
    let evaluated = chart
        .mark(WidgetCell::widget(Slider::new("amount", 0.0, 10.0).title("Amount")).name("controls"))
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();
    let width = evaluated
        .widget_frames
        .by_mark_path
        .values()
        .find(|frame| frame.widget_id == "amount")
        .unwrap()
        .bounds
        .width;
    (width, evaluated)
}

#[tokio::test]
async fn slider_widget_cell_stretches_in_auto_and_flex_tracks() {
    for track in [None, Some(TrackSizing::Flex(1.0))] {
        let (width, evaluated) = slider_cell_frame_width(track).await;
        assert_eq!(width, 300.0);
        let slider = find_group(&evaluated.scene_graph.marks, "amount").unwrap();
        let track = find_rect(&slider.marks, "track").unwrap();
        assert!(track.x2_vec()[0] > 250.0);
    }
}

#[tokio::test]
async fn slider_widget_cell_overflows_rigid_px_track_at_intrinsic_minimum() {
    let (width, evaluated) = slider_cell_frame_width(Some(TrackSizing::Px(40.0))).await;
    assert_eq!(width, 80.0);
    let slider = find_group(&evaluated.scene_graph.marks, "amount").unwrap();
    assert!(find_rect(&slider.marks, "track").unwrap().x2_vec()[0] > 40.0);

    let (wide_width, _) = slider_cell_frame_width(Some(TrackSizing::Px(200.0))).await;
    assert_eq!(wide_width, 80.0);
}

#[tokio::test]
async fn slider_widget_cell_stretches_across_grid_span() {
    let ctx = SessionContext::new();
    let evaluated = Chart::<GridConcat>::new()
        .configure_coord(|grid| {
            grid.rows(1)
                .columns(2)
                .column_widths([TrackSizing::Flex(1.0), TrackSizing::Flex(1.0)])
        })
        .plot_size(300.0, 80.0)
        .mark(
            WidgetCell::widget(Slider::new("amount", 0.0, 10.0).title("Amount"))
                .name("controls")
                .at(0, 0)
                .grid_column_span(2),
        )
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();
    let frame = evaluated
        .widget_frames
        .by_mark_path
        .values()
        .find(|frame| frame.widget_id == "amount")
        .unwrap();
    assert_eq!(frame.bounds.width, 300.0);
}

fn find_rect<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneRectMark> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Rect(rect) if rect.name == name => Some(rect),
        SceneMark::Group(group) => find_rect(&group.marks, name),
        _ => None,
    })
}

fn find_symbol<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneSymbolMark> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Symbol(symbol) if symbol.name == name => Some(symbol),
        SceneMark::Group(group) => find_symbol(&group.marks, name),
        _ => None,
    })
}

#[tokio::test]
async fn slider_round_trips_normalized_value_and_drag_contract() {
    let ctx = SessionContext::new();
    let compiled = Chart::<Cartesian>::new()
        .widget(
            Slider::new("fare", 5.0, 25.0)
                .step(4.0)
                .default(12.0)
                .title("Fare")
                .format(".0f")
                .throttle_ms(16)
                .position(ChromePosition::Left),
        )
        .compile(&ctx)
        .await
        .unwrap();
    let compiled: avenger_chart::plot::CompiledPlot =
        bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();

    assert_eq!(
        compiled.get_default_params().get("fare__value"),
        Some(&ScalarValue::Float64(Some(13.0)))
    );
    let down = compiled
        .event_bindings()
        .iter()
        .find(|binding| binding.event_type == ChartEventType::MouseDown)
        .unwrap();
    assert_eq!(down.mark_ids(), &["fare.track", "fare.fill", "fare.handle"]);
    assert!(!down.consume);
    let drag = compiled
        .event_bindings()
        .iter()
        .find(|binding| {
            binding.event_type == ChartEventType::CursorMoved && binding.between.is_some()
        })
        .unwrap();
    assert_eq!(drag.throttle_ms, Some(16));
    assert!(!drag.consume);
    assert_eq!(
        drag.between.as_ref().unwrap().start.mark_ids(),
        &["fare.track", "fare.fill", "fare.handle"]
    );
    assert!(drag.between.as_ref().unwrap().end.mark_ids().is_empty());
}

#[tokio::test]
async fn slider_css_drives_frame_inner_track_and_handle_geometry() {
    let ctx = SessionContext::new();
    let mut theme = Theme::light();
    theme
        .append_css(
            r#"
            slider#volume { height: 40px; padding-inline: 5px; }
            slider#volume::part(track) {
                slider-min-width: 120px;
                slider-track-height: 4px;
            }
            slider#volume::part(handle) { slider-handle-size: 20px; }
            slider#volume::part(value-label) { slider-value-padding: 16px; }
            "#,
        )
        .unwrap();
    let evaluated = Chart::<Cartesian>::new()
        .theme(theme)
        .widget(
            Slider::new("volume", 0.0, 100.0)
                .step(10.0)
                .default(25.0)
                .title("Volume")
                .format(".0f")
                .position(ChromePosition::Left),
        )
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let group = find_group(&evaluated.scene_graph.marks, "volume").unwrap();
    assert_eq!(group.clip, Clip::None);
    let frame = evaluated
        .widget_frames
        .by_mark_path
        .values()
        .find(|frame| frame.widget_id == "volume")
        .unwrap();
    assert_eq!((frame.bounds.width, frame.bounds.height), (120.0, 40.0));
    assert_eq!(
        frame
            .runtime_inputs
            .get("__widget_style_host_padding_inline"),
        Some(&ScalarValue::Float32(Some(5.0)))
    );

    let track = find_rect(&group.marks, "track").unwrap();
    assert_eq!(track.x_vec(), vec![15.0]);
    assert_eq!(track.x2_vec(), vec![105.0]);
    assert_eq!(track.y_vec(), vec![28.0]);
    assert_eq!(track.y2_vec(), vec![32.0]);
    let fill = find_rect(&group.marks, "fill").unwrap();
    assert_eq!(fill.x2_vec(), vec![42.0]);
    let handle = find_symbol(&group.marks, "handle").unwrap();
    assert_eq!(handle.x_vec(), vec![42.0]);
    assert_eq!(handle.y_vec(), vec![30.0]);
    assert_eq!(handle.size.as_vec(1, None), vec![400.0]);
}

#[tokio::test]
async fn slider_collapses_nonpositive_inner_track_to_frame_center() {
    let ctx = SessionContext::new();
    let mut theme = Theme::light();
    theme
        .append_css("slider#volume { padding-inline: 50px; }")
        .unwrap();
    let evaluated = Chart::<Cartesian>::new()
        .theme(theme)
        .widget(
            Slider::new("volume", 0.0, 100.0)
                .default(40.0)
                .title("Volume")
                .format(".0f")
                .position(ChromePosition::Left),
        )
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();
    let frame = evaluated
        .widget_frames
        .by_mark_path
        .values()
        .find(|frame| frame.widget_id == "volume")
        .unwrap();
    let center = frame.bounds.width / 2.0;
    let group = find_group(&evaluated.scene_graph.marks, "volume").unwrap();
    let track = find_rect(&group.marks, "track").unwrap();
    assert_eq!(track.x_vec(), vec![center]);
    assert_eq!(track.x2_vec(), vec![center]);
    let fill = find_rect(&group.marks, "fill").unwrap();
    assert_eq!(fill.x_vec(), vec![center]);
    assert_eq!(fill.x2_vec(), vec![center]);
    assert_eq!(
        find_symbol(&group.marks, "handle").unwrap().x_vec(),
        vec![center]
    );
}

#[tokio::test]
async fn slider_rejects_invalid_domains_steps_params_and_formats() {
    let ctx = SessionContext::new();
    for slider in [
        Slider::new("bounds", 1.0, 1.0),
        Slider::new("step", 0.0, 1.0).step(0.0),
        Slider::new("default", 0.0, 1.0).default(f64::NAN),
        Slider::new("format", 0.0, 1.0).format("not-a-format"),
        Slider::new("param", 0.0, 1.0)
            .value_param(avenger_chart::prelude::Param::new("bad", 0_i64)),
        Slider::new("1leading_digit", 0.0, 1.0),
        Slider::new("param_name", 0.0, 1.0)
            .value_param(avenger_chart::prelude::Param::new("1bad", 0.0)),
    ] {
        assert!(
            Chart::<Cartesian>::new()
                .widget(slider.position(ChromePosition::Left))
                .compile(&ctx)
                .await
                .is_err()
        );
    }
}
