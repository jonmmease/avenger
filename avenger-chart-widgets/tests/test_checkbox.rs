use avenger_chart::prelude::{
    Cartesian, Chart, ChartEventType, ChartWidgetPlacementExt, ChromePosition, EvaluationRequest,
    Theme,
};
use avenger_chart_widgets::Checkbox;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

fn find_rule<'a>(
    marks: &'a [SceneMark],
    name: &str,
) -> Option<&'a avenger_scenegraph::marks::rule::SceneRuleMark> {
    for mark in marks {
        match mark {
            SceneMark::Rule(rule) if rule.name == name => return Some(rule),
            SceneMark::Group(group) => {
                if let Some(rule) = find_rule(&group.marks, name) {
                    return Some(rule);
                }
            }
            _ => {}
        }
    }
    None
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

fn find_rect<'a>(
    marks: &'a [SceneMark],
    name: &str,
) -> Option<&'a avenger_scenegraph::marks::rect::SceneRectMark> {
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
async fn checkbox_round_trips_and_checked_param_controls_inset_vector() {
    let ctx = SessionContext::new();
    let compiled = Chart::<Cartesian>::new()
        .widget(Checkbox::new("regions", "Regions", false).position(ChromePosition::Left))
        .compile(&ctx)
        .await
        .unwrap();
    let compiled: avenger_chart::plot::CompiledPlot =
        bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();

    assert_eq!(
        compiled.get_default_params().get("regions__checked"),
        Some(&ScalarValue::Boolean(Some(false)))
    );
    assert_eq!(compiled.cursor_params(), &["regions__cursor"]);
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
        assert_eq!(
            binding.mark_ids(),
            &["regions.box", "regions.check", "regions.label"]
        );
    }

    let unchecked = compiled.evaluate(&ctx, None).await.unwrap();
    assert!(find_rule(&unchecked.scene_graph.marks, "check").is_none());

    let mut params = IndexMap::new();
    params.insert(
        "regions__checked".to_string(),
        ScalarValue::Boolean(Some(true)),
    );
    let mut session = std::sync::Arc::new(compiled).instantiate(std::sync::Arc::new(ctx));
    let checked = session
        .evaluate(EvaluationRequest::new().params(params))
        .await
        .unwrap();
    let check = find_rule(&checked.scene_graph.marks, "check").unwrap();
    assert_eq!(check.len, 2);

    let xs = check.x.as_vec(check.len as usize, check.indices.as_ref());
    let ys = check.y.as_vec(check.len as usize, check.indices.as_ref());
    let x2s = check.x2.as_vec(check.len as usize, check.indices.as_ref());
    let y2s = check.y2.as_vec(check.len as usize, check.indices.as_ref());
    let min_x = xs.iter().chain(&x2s).copied().fold(f32::INFINITY, f32::min);
    let max_x = xs
        .iter()
        .chain(&x2s)
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = ys.iter().chain(&y2s).copied().fold(f32::INFINITY, f32::min);
    let max_y = ys
        .iter()
        .chain(&y2s)
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(min_x > 0.0 && max_x < 14.0, "x bounds: {min_x}..{max_x}");
    assert!(min_y > 9.0 && max_y < 23.0, "y bounds: {min_y}..{max_y}");
}

#[tokio::test]
async fn checkbox_css_geometry_drives_frame_and_parts() {
    let ctx = SessionContext::new();
    let mut theme = Theme::light();
    theme
        .append_css(
            "checkbox { height: 40px; } \
             checkbox::part(box) { choice-control-size: 18px; } \
             checkbox::part(label) { control-label-gap: 12px; } \
             checkbox::part(focus-ring) { focus-gap: 3px; }",
        )
        .unwrap();
    let evaluated = Chart::<Cartesian>::new()
        .theme(theme)
        .widget(Checkbox::new("custom", "Custom", true).position(ChromePosition::Left))
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let group = find_group(&evaluated.scene_graph.marks, "custom").unwrap();
    assert!(matches!(group.clip, Clip::Rect { height, .. } if height == 40.0));

    let box_mark = find_rect(&group.marks, "box").unwrap();
    assert_eq!(box_mark.x_vec(), vec![0.0]);
    assert_eq!(box_mark.x2_vec(), vec![18.0]);
    assert_eq!(box_mark.y_vec(), vec![11.0]);
    assert_eq!(box_mark.y2_vec(), vec![29.0]);

    let focus = find_rect(&group.marks, "focus-ring").unwrap();
    assert_eq!(focus.x_vec(), vec![-3.0]);
    assert_eq!(focus.x2_vec(), vec![21.0]);
    assert_eq!(focus.y_vec(), vec![8.0]);
    assert_eq!(focus.y2_vec(), vec![32.0]);
    assert!(!focus.interactive);

    let label = group
        .marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Text(text) if text.name == "label" => Some(text),
            _ => None,
        })
        .unwrap();
    assert_eq!(label.x.as_vec(1, None), vec![30.0]);
    assert_eq!(label.y.as_vec(1, None), vec![20.0]);
}
