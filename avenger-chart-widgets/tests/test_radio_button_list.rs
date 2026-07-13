use std::sync::Arc;

use avenger_chart::prelude::{
    AvengerChartError, Cartesian, Chart, ChartWidgetPlacementExt, ChromePosition,
    EvaluationRequest, Param, Theme, WidgetItemRow, WidgetItems,
};
use avenger_chart_widgets::RadioButtonList;
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, rect::SceneRectMark, symbol::SceneSymbolMark,
};
use datafusion::{
    common::ScalarValue,
    prelude::{SessionContext, col, lit},
};
use indexmap::IndexMap;

fn static_items() -> WidgetItems {
    WidgetItems::Static(vec![
        WidgetItemRow::new([
            ("code".to_string(), ScalarValue::Int64(Some(1))),
            (
                "name".to_string(),
                ScalarValue::Utf8(Some("North".to_string())),
            ),
        ]),
        WidgetItemRow::new([
            ("code".to_string(), ScalarValue::Int64(Some(2))),
            (
                "name".to_string(),
                ScalarValue::Utf8(Some("South".to_string())),
            ),
        ]),
    ])
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
async fn radio_list_round_trips_and_rejects_absent_static_param_values() {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Chart::<Cartesian>::new()
        .widget(
            RadioButtonList::new("regions", static_items())
                .item_value(col("code"))
                .label(col("name"))
                .position(ChromePosition::Left),
        )
        .compile(ctx.as_ref())
        .await
        .expect("compile static radio list");
    assert_eq!(
        compiled.get_default_params().get("regions__value"),
        Some(&ScalarValue::Int64(Some(1)))
    );

    let restored = bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();
    for candidate in [compiled, restored] {
        candidate
            .evaluate(ctx.as_ref(), None)
            .await
            .expect("default is present");
        let mut session = Arc::new(candidate).instantiate(ctx.clone());
        let mut patch = IndexMap::new();
        patch.insert("regions__value".to_string(), ScalarValue::Int64(Some(99)));
        let error = match session
            .evaluate(EvaluationRequest::new().exact().param_patch(patch))
            .await
        {
            Ok(_) => panic!("absent external value must fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            AvengerChartError::InvalidWidgetItems { widget_id, role, message }
                if widget_id == "regions"
                    && role == "radio-button-list selected value"
                    && message.contains("Int64(99)")
        ));
    }
}

#[tokio::test]
async fn radio_list_requires_dataframe_default_and_revalidates_item_revisions() {
    let ctx = Arc::new(SessionContext::new());
    let include_south = Param::new("include_south", true);
    let data = ctx
        .sql("SELECT * FROM (VALUES (1, 'North'), (2, 'South')) AS t(code, name)")
        .await
        .unwrap()
        .filter(include_south.expr().or(col("code").eq(lit(1_i64))))
        .unwrap();
    let items = WidgetItems::DataFrame {
        data,
        order_key: vec![col("code")],
    };

    let no_default = Chart::<Cartesian>::new()
        .param(include_south.clone())
        .widget(
            RadioButtonList::new("regions", items.clone())
                .item_value(col("code"))
                .label(col("name"))
                .position(ChromePosition::Left),
        )
        .compile(ctx.as_ref())
        .await;
    assert!(matches!(
        no_default,
        Err(AvengerChartError::InvalidArgument(message))
            if message.contains("requires .default")
                && message.contains("DataFrame item sources")
    ));

    let compiled = Chart::<Cartesian>::new()
        .param(include_south)
        .widget(
            RadioButtonList::new("regions", items)
                .item_value(col("code"))
                .label(col("name"))
                .default(2_i64)
                .position(ChromePosition::Left),
        )
        .compile(ctx.as_ref())
        .await
        .expect("compile dynamic radio list");
    let restored = bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();
    for candidate in [compiled, restored] {
        let candidate = Arc::new(candidate);
        let mut invalid_value_session = candidate.clone().instantiate(ctx.clone());
        invalid_value_session
            .evaluate(EvaluationRequest::new().exact())
            .await
            .expect("selected item initially exists");
        let mut invalid_value_patch = IndexMap::new();
        invalid_value_patch.insert("regions__value".to_string(), ScalarValue::Int64(Some(99)));
        let error = match invalid_value_session
            .evaluate(
                EvaluationRequest::new()
                    .exact()
                    .param_patch(invalid_value_patch),
            )
            .await
        {
            Ok(_) => panic!("absent dynamic value must fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            AvengerChartError::InvalidWidgetItems { widget_id, role, message }
                if widget_id == "regions"
                    && role == "radio-button-list selected value"
                    && message.contains("Int64(99)")
        ));

        let mut revision_session = candidate.instantiate(ctx.clone());
        revision_session
            .evaluate(EvaluationRequest::new().exact())
            .await
            .expect("selected item initially exists");
        let mut patch = IndexMap::new();
        patch.insert(
            "include_south".to_string(),
            ScalarValue::Boolean(Some(false)),
        );
        let error = match revision_session
            .evaluate(EvaluationRequest::new().exact().param_patch(patch))
            .await
        {
            Ok(_) => panic!("removing the selected item must fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            AvengerChartError::InvalidWidgetItems { widget_id, role, message }
                if widget_id == "regions"
                    && role == "radio-button-list default value"
                    && message.contains("Int64(2)")
        ));
    }
}

#[tokio::test]
async fn radio_list_css_geometry_keeps_rows_controls_and_selected_layers_coincident() {
    let ctx = SessionContext::new();
    let mut theme = Theme::light();
    theme
        .append_css(
            "radio-button-list#regions { height: 40px; item-gap: 12px; } \
             radio-button-list#regions::part(control) { \
                 choice-control-size: 18px; radio-center-size: 6px; \
                 radio-selected-border-width: 3px; \
             } \
             radio-button-list#regions::part(label) { control-label-gap: 11px; } \
             radio-button-list#regions::part(focus-ring) { focus-gap: 4px; }",
        )
        .unwrap();
    let evaluated = Chart::<Cartesian>::new()
        .theme(theme)
        .widget(
            RadioButtonList::new("regions", static_items())
                .item_value(col("code"))
                .label(col("name"))
                .default(2_i64)
                .position(ChromePosition::Left),
        )
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let group = find_group(&evaluated.scene_graph.marks, "regions").unwrap();
    let row = find_rect(&group.marks, "row").unwrap();
    assert_eq!(row.y_vec(), vec![0.0, 52.0]);
    assert_eq!(row.height_iter().collect::<Vec<_>>(), vec![40.0, 40.0]);
    assert!(row.interactive);

    let control = find_symbol(&group.marks, "control").unwrap();
    assert_eq!(control.x_vec(), vec![9.0, 9.0]);
    assert_eq!(control.y_vec(), vec![20.0, 72.0]);
    let control_sizes = control.size_vec();
    assert_eq!(control_sizes.len(), 2);
    assert_eq!(control_sizes[0], control_sizes[1]);

    let selected = find_symbol(&group.marks, "selected-control").unwrap();
    assert_eq!(selected.x_vec(), vec![9.0]);
    assert_eq!(selected.y_vec(), vec![72.0]);
    assert_eq!(selected.size_vec(), vec![control_sizes[1]]);
    assert_eq!(selected.stroke_width, Some(3.0));
    assert!(!selected.interactive);

    let center = find_symbol(&group.marks, "center").unwrap();
    assert_eq!(center.x_vec(), vec![9.0]);
    assert_eq!(center.y_vec(), vec![72.0]);
    assert_eq!(center.size_vec().len(), 1);
    assert!(center.size_vec()[0] < control_sizes[1]);

    let focus = find_symbol(&group.marks, "focus-ring").unwrap();
    assert_eq!(focus.x_vec(), vec![9.0, 9.0]);
    assert_eq!(focus.y_vec(), vec![20.0, 72.0]);
    assert_eq!(focus.size_vec().len(), 2);
    assert_eq!(focus.size_vec()[0], focus.size_vec()[1]);
    assert!(focus.size_vec()[0] > control_sizes[0]);
    assert!(!focus.interactive);
}
