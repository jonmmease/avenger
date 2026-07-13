use std::sync::Arc;

use avenger_chart::{
    pixel_frame::{PixelFrameRectPositionChannels, PixelFrameTextPositionChannels},
    prelude::*,
};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::{
    array::{Int64Array, RecordBatch},
    datatypes::{DataType, Field, Schema},
};

fn find_group<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a [SceneMark]> {
    find_scene_group(marks, name).map(|group| group.marks.as_slice())
}

fn find_scene_group<'a>(
    marks: &'a [SceneMark],
    name: &str,
) -> Option<&'a avenger_scenegraph::marks::group::SceneGroup> {
    for mark in marks {
        if let SceneMark::Group(group) = mark {
            if group.name == name {
                return Some(group);
            }
            if let Some(found) = find_scene_group(&group.marks, name) {
                return Some(found);
            }
        }
    }
    None
}

fn find_rect_path(marks: &[SceneMark], name: &str, prefix: &mut Vec<usize>) -> Option<Vec<usize>> {
    for (index, mark) in marks.iter().enumerate() {
        prefix.push(index);
        if matches!(mark, SceneMark::Rect(rect) if rect.name == name) {
            return Some(prefix.clone());
        }
        if let SceneMark::Group(group) = mark
            && let Some(path) = find_rect_path(&group.marks, name, prefix)
        {
            return Some(path);
        }
        prefix.pop();
    }
    None
}

#[derive(Clone)]
struct ContractWidget;

impl ChartWidget for ContractWidget {
    fn id(&self) -> &str {
        "contract"
    }

    fn kind(&self) -> &'static str {
        "contract-widget"
    }

    fn expand(
        &self,
        _ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        Ok(WidgetExpansion {
            expansion: ToolExpansion::new()
                .mark(
                    Rect::<PixelFrame>::new()
                        .id("box")
                        .x(0.0)
                        .x2(80.0)
                        .y(0.0)
                        .y2(24.0),
                )
                .mark(
                    Text::<PixelFrame>::new()
                        .id("label")
                        .x(8.0)
                        .y(12.0)
                        .text("Contract"),
                )
                .mark(
                    Rect::<PixelFrame>::new()
                        .id("focus-ring")
                        .x(0.0)
                        .x2(80.0)
                        .y(0.0)
                        .y2(24.0),
                )
                .event_binding(ChartEventBinding::on_between_end(
                    ChartEventStream::on(ChartEventType::MouseDown).mark("contract.box"),
                    ChartEventStream::on(ChartEventType::MouseUp).mark("contract.box"),
                ))
                .event_binding(ChartEventBinding::on(ChartEventType::Click)),
            items: Some(WidgetItems::Static(vec![
                WidgetItemRow::new(vec![(
                    "value".to_string(),
                    datafusion::common::ScalarValue::Utf8(Some("one".to_string())),
                )]),
                WidgetItemRow::new(vec![(
                    "value".to_string(),
                    datafusion::common::ScalarValue::Utf8(Some("two".to_string())),
                )]),
            ])),
            measure: WidgetMeasureSpec {
                width: WidgetAxisMeasureSpec::Content {
                    expr: WidgetMeasureExpr::Add(vec![
                        WidgetMeasureExpr::TextExtent {
                            part: "label".to_string(),
                            axis: WidgetTextMeasureAxis::Width,
                            data_encoded: false,
                        },
                        WidgetMeasureExpr::StyleLength {
                            part: Some("box".to_string()),
                            property: WidgetStyleProperty::StrokeWidth,
                        },
                    ]),
                    min_px: 24.0,
                    max_px: Some(200.0),
                },
                height: WidgetAxisMeasureSpec::Fixed { px: 24.0 },
            },
        })
    }
}

#[derive(Clone)]
struct DecorativeTargetWidget;

impl ChartWidget for DecorativeTargetWidget {
    fn id(&self) -> &str {
        "decorative-target"
    }

    fn kind(&self) -> &'static str {
        "decorative-target-widget"
    }

    fn expand(
        &self,
        _ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        Ok(WidgetExpansion {
            expansion: ToolExpansion::new()
                .mark(
                    Rect::<PixelFrame>::new()
                        .id("focus-ring")
                        .x(0.0)
                        .x2(24.0)
                        .y(0.0)
                        .y2(24.0),
                )
                .event_binding(
                    ChartEventBinding::on(ChartEventType::Click)
                        .mark("decorative-target.focus-ring"),
                ),
            items: None,
            measure: WidgetMeasureSpec::fixed(24.0, 24.0),
        })
    }
}

#[derive(Clone)]
struct ContractNativeWidget;

impl NativeWidget for ContractNativeWidget {
    fn id(&self) -> &str {
        "native-contract"
    }

    fn kind(&self) -> &'static str {
        "native-contract-widget"
    }

    fn schema_version(&self) -> u32 {
        3
    }

    fn payload(&self) -> serde_json::Value {
        serde_json::json!({
            "z": null,
            "config": {"enabled": true, "threshold": 2.5},
            "labels": ["one", "two"]
        })
    }

    fn measure(&self) -> NativeWidgetMeasureSpec {
        NativeWidgetMeasureSpec::Registry
    }

    fn state(&self) -> NativeWidgetStateSpec {
        NativeWidgetStateSpec::try_new(vec![CompiledParamSpec::shared(&Param::new(
            "native-contract-value",
            "ready",
        ))])
        .unwrap()
    }
}

#[derive(Clone)]
struct DataFrameContractWidget {
    data: DataFrame,
}

impl ChartWidget for DataFrameContractWidget {
    fn id(&self) -> &str {
        "data-contract"
    }

    fn kind(&self) -> &'static str {
        "data-contract-widget"
    }

    fn expand(
        &self,
        _ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        Ok(WidgetExpansion {
            expansion: ToolExpansion::new(),
            items: Some(WidgetItems::DataFrame {
                data: self.data.clone(),
                order_key: vec![col("value")],
            }),
            measure: WidgetMeasureSpec::fixed(40.0, 20.0),
        })
    }
}

#[tokio::test]
async fn composed_widget_schema_round_trips_with_symbolic_measurement() {
    let ctx = datafusion::prelude::SessionContext::new();
    let compiled = Chart::<Cartesian>::new()
        .widget(ContractWidget.position(ChromePosition::Right))
        .compile(&ctx)
        .await
        .unwrap();

    let bytes = bincode::serialize(&compiled).unwrap();
    let decoded: avenger_chart::plot::CompiledPlot = bincode::deserialize(&bytes).unwrap();
    assert_eq!(bytes, bincode::serialize(&decoded).unwrap());
    let attachment = decoded.widgets().first().unwrap();
    let artifact_json = serde_json::to_string(&attachment.widget).unwrap();
    for runtime_only in ["resolved_style", "presentation_state", "css_sources"] {
        assert!(!artifact_json.contains(runtime_only));
    }
    assert_eq!(attachment.declaration_order, 0);
    assert_eq!(
        attachment.placement,
        WidgetPlacement::Guide(ChromePosition::Right)
    );
    let CompiledWidget::Composed(widget) = &attachment.widget else {
        panic!("expected composed widget")
    };
    assert_eq!(widget.id, "contract");
    assert_eq!(widget.kind, "contract-widget");
    assert_eq!(widget.marks.len(), 3);
    assert_eq!(
        widget.marks[0]
            .state()
            .widget_theme
            .as_ref()
            .expect("widget theme provenance")
            .part,
        "box"
    );
    assert_eq!(widget.relative_target_paths["contract.box"], vec![vec![0]]);
    assert_eq!(
        widget.relative_target_paths["contract.label"],
        vec![vec![1]]
    );
    assert_eq!(
        widget.relative_target_paths["contract.focus-ring"],
        vec![vec![2]]
    );
    let items = widget.items.as_ref().expect("compiled item plan");
    assert_eq!(items.order_column, "__order");
    assert_eq!(items.validations.len(), 1);
    assert!(matches!(
        widget.measure.width,
        WidgetAxisMeasureSpec::Content {
            expr: WidgetMeasureExpr::Add(_),
            ..
        }
    ));
    let evaluated = decoded.evaluate(&ctx, None).await.unwrap();
    let widget_children =
        find_group(&evaluated.scene_graph.marks, "contract").expect("compiler-owned widget group");
    let widget_group = find_scene_group(&evaluated.scene_graph.marks, "contract").unwrap();
    assert!(matches!(
        widget_group.clip,
        avenger_scenegraph::marks::group::Clip::Rect { width, height, .. }
            if width > 24.0 && width <= 200.0 && height == 24.0
    ));
    assert!(
        widget_children
            .iter()
            .any(|mark| matches!(mark, SceneMark::Rect(rect) if rect.name == "box"))
    );
    assert!(
        widget_children
            .iter()
            .any(|mark| matches!(mark, SceneMark::Text(text) if text.name == "label"))
    );
    let drag_binding = decoded
        .event_bindings()
        .iter()
        .find(|binding| binding.between.is_some())
        .expect("widget drag binding");
    assert!(drag_binding.mark_ids().is_empty());
    let resolved = drag_binding
        .between
        .as_ref()
        .unwrap()
        .start
        .resolved_mark_paths()
        .unwrap();
    assert_eq!(resolved, &[vec![0, 0]]);
    let box_path = find_rect_path(&evaluated.scene_graph.marks, "box", &mut Vec::new()).unwrap();
    assert!(box_path.ends_with(&resolved[0]));

    let click_binding = decoded
        .event_bindings()
        .iter()
        .find(|binding| binding.event_type == ChartEventType::Click)
        .expect("widget click binding");
    assert_eq!(
        click_binding.mark_ids(),
        &["contract.box".to_string(), "contract.label".to_string()]
    );
    assert_eq!(
        click_binding.resolved_mark_paths().unwrap(),
        &[vec![0, 0], vec![0, 1]]
    );
}

#[tokio::test]
async fn composed_widget_rejects_decorative_event_targets() {
    let ctx = datafusion::prelude::SessionContext::new();
    let result = Chart::<Cartesian>::new()
        .widget(DecorativeTargetWidget.position(ChromePosition::Right))
        .compile(&ctx)
        .await;
    assert!(matches!(
        result,
        Err(AvengerChartError::InvalidArgument(message))
            if message.contains("cannot target decorative part 'focus-ring'")
    ));
}

#[tokio::test]
async fn native_variants_round_trip_and_fail_structurally_before_w5() {
    let ctx = datafusion::prelude::SessionContext::new();
    let guide = Chart::<Cartesian>::new()
        .native_widget(ContractNativeWidget.position(ChromePosition::Bottom))
        .compile(&ctx)
        .await
        .unwrap();
    let explicit = Chart::<PixelFrame>::new()
        .host_native_widget(ContractNativeWidget)
        .compile(&ctx)
        .await
        .unwrap();

    for (compiled, placement) in [
        (&guide, WidgetPlacement::Guide(ChromePosition::Bottom)),
        (&explicit, WidgetPlacement::ExplicitFrame),
    ] {
        let bytes = bincode::serialize(compiled).unwrap();
        let decoded: avenger_chart::plot::CompiledPlot = bincode::deserialize(&bytes).unwrap();
        let attachment = decoded.widgets().first().unwrap();
        assert_eq!(attachment.placement, placement);
        let CompiledWidget::Native(widget) = &attachment.widget else {
            panic!("expected native widget")
        };
        assert_eq!(widget.schema_version, 3);
        assert_eq!(widget.state.params().len(), 1);
        assert_eq!(
            widget.payload.as_str(),
            r#"{"config":{"enabled":true,"threshold":2.5},"labels":["one","two"],"z":null}"#
        );
        assert!(matches!(
            decoded.evaluate(&ctx, None).await,
            Err(AvengerChartError::NativeWidgetRuntimeUnavailable { widget_id, kind })
                if widget_id == "native-contract" && kind == "native-contract-widget"
        ));
        let session_ctx = Arc::new(datafusion::prelude::SessionContext::new());
        let mut session = Arc::new(decoded.clone()).instantiate(session_ctx);
        assert!(matches!(
            session.evaluate(EvaluationRequest::new()).await,
            Err(AvengerChartError::NativeWidgetRuntimeUnavailable { widget_id, kind })
                if widget_id == "native-contract" && kind == "native-contract-widget"
        ));
    }
}

#[tokio::test]
async fn dataframe_widget_items_compile_total_order_projection() {
    let ctx = datafusion::prelude::SessionContext::new();
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(Int64Array::from(vec![20_i64, 10_i64]))],
    )
    .unwrap();
    let data = ctx.read_batch(batch).unwrap();
    let compiled = Chart::<Cartesian>::new()
        .widget(DataFrameContractWidget { data }.position(ChromePosition::Left))
        .compile(&ctx)
        .await
        .unwrap();
    let bytes = bincode::serialize(&compiled).unwrap();
    let decoded: avenger_chart::plot::CompiledPlot = bincode::deserialize(&bytes).unwrap();
    let CompiledWidget::Composed(widget) = &decoded.widgets()[0].widget else {
        panic!("expected composed widget")
    };
    let plan = widget.items.as_ref().unwrap();
    assert_eq!(plan.order_column, "__order");
    assert_eq!(plan.validations.len(), 1);
    let restored = plan.data.dataframe_with_context(&ctx).unwrap();
    for column in ["__widget_order_key_0", "__order", "__idx"] {
        assert!(
            restored
                .schema()
                .field_with_unqualified_name(column)
                .is_ok()
        );
    }
}
