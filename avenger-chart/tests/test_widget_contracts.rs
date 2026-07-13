use std::sync::Arc;

use avenger_chart::{
    bake::{BakeContextId, BakePolicy, ContextBakeStatus},
    pixel_frame::{
        PixelFrameRectPositionChannels, PixelFrameSymbolPositionChannels,
        PixelFrameTextPositionChannels,
    },
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

fn find_symbol<'a>(
    marks: &'a [SceneMark],
    name: &str,
) -> Option<&'a avenger_scenegraph::marks::symbol::SceneSymbolMark> {
    for mark in marks {
        match mark {
            SceneMark::Symbol(symbol) if symbol.name == name => return Some(symbol),
            SceneMark::Group(group) => {
                if let Some(symbol) = find_symbol(&group.marks, name) {
                    return Some(symbol);
                }
            }
            _ => {}
        }
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
                        .x2(col("box_x2"))
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
                .event_binding(
                    ChartEventBinding::on(ChartEventType::Click)
                        .filter(avenger_chart::event::datum("value").is_not_null()),
                ),
            items: Some(WidgetItems::Static(vec![
                WidgetItemRow::new(vec![
                    (
                        "value".to_string(),
                        datafusion::common::ScalarValue::Utf8(Some("one".to_string())),
                    ),
                    (
                        "box_x2".to_string(),
                        datafusion::common::ScalarValue::Float32(Some(80.0)),
                    ),
                ]),
                WidgetItemRow::new(vec![
                    (
                        "value".to_string(),
                        datafusion::common::ScalarValue::Utf8(Some("two".to_string())),
                    ),
                    (
                        "box_x2".to_string(),
                        datafusion::common::ScalarValue::Float32(Some(80.0)),
                    ),
                ]),
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
            presentation: WidgetPresentationBindings::default(),
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
            presentation: WidgetPresentationBindings::default(),
        })
    }
}

#[derive(Clone)]
struct FixedContractWidget {
    id: &'static str,
    height: f32,
}

impl ChartWidget for FixedContractWidget {
    fn id(&self) -> &str {
        self.id
    }

    fn kind(&self) -> &'static str {
        "fixed-contract-widget"
    }

    fn expand(
        &self,
        _ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        Ok(WidgetExpansion {
            expansion: ToolExpansion::new().mark(
                Rect::<PixelFrame>::new()
                    .id("box")
                    .x(0.0)
                    .x2(40.0)
                    .y(0.0)
                    .y2(self.height),
            ),
            items: None,
            measure: WidgetMeasureSpec::fixed(40.0, self.height),
            presentation: WidgetPresentationBindings::default(),
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

#[derive(Clone)]
struct DataFrameScaledWidget {
    data: DataFrame,
}

#[derive(Clone)]
struct ExplicitPartDataWidget {
    data: DataFrame,
}

#[derive(Clone)]
struct ThemedGeometryWidget;

#[derive(Clone)]
struct ScaledContractWidget {
    id: &'static str,
    categories: [&'static str; 2],
    amounts: [f64; 2],
}

impl ChartWidget for ScaledContractWidget {
    fn id(&self) -> &str {
        self.id
    }

    fn kind(&self) -> &'static str {
        "scaled-contract-widget"
    }

    fn expand(
        &self,
        _ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        let items = self
            .categories
            .iter()
            .zip(self.amounts)
            .map(|(category, amount)| {
                WidgetItemRow::new(vec![
                    (
                        "category".to_string(),
                        datafusion::common::ScalarValue::Utf8(Some((*category).to_string())),
                    ),
                    (
                        "amount".to_string(),
                        datafusion::common::ScalarValue::Float64(Some(amount)),
                    ),
                ])
            })
            .collect();
        Ok(WidgetExpansion {
            expansion: ToolExpansion::new().mark(
                Symbol::<PixelFrame>::new()
                    .id("dot")
                    .x(20.0)
                    .y(12.0)
                    .fill(col("category"))
                    .size(col("amount")),
            ),
            items: Some(WidgetItems::Static(items)),
            measure: WidgetMeasureSpec::fixed(40.0, 24.0),
            presentation: WidgetPresentationBindings::default(),
        })
    }
}

impl ChartWidget for ThemedGeometryWidget {
    fn id(&self) -> &str {
        "themed-geometry"
    }

    fn kind(&self) -> &'static str {
        "themed-geometry-widget"
    }

    fn expand(
        &self,
        ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        Ok(WidgetExpansion {
            expansion: ToolExpansion::new().mark(
                Rect::<PixelFrame>::new()
                    .id("box")
                    .x(0.0)
                    .x2(ctx.part_style("box", WidgetStyleProperty::ChoiceControlSize))
                    .y(0.0)
                    .y2(ctx.frame_height()),
            ),
            items: None,
            measure: WidgetMeasureSpec::fixed(40.0, 23.0),
            presentation: WidgetPresentationBindings::default()
                .checked(datafusion::prelude::lit(true)),
        })
    }
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
            presentation: WidgetPresentationBindings::default(),
        })
    }
}

impl ChartWidget for DataFrameScaledWidget {
    fn id(&self) -> &str {
        "revision-scale"
    }

    fn kind(&self) -> &'static str {
        "scaled-contract-widget"
    }

    fn expand(
        &self,
        ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        Ok(WidgetExpansion {
            expansion: ToolExpansion::new()
                .mark(
                    Symbol::<PixelFrame>::new()
                        .id("dot")
                        .x(20.0)
                        .y(12.0)
                        .fill(col("category"))
                        .size(col("amount")),
                )
                .mark(
                    Symbol::<PixelFrame>::new()
                        .id("paint")
                        .x(24.0)
                        .y(12.0)
                        .fill(ctx.part_style("paint", WidgetStyleProperty::Fill))
                        .size(20.0),
                ),
            items: Some(WidgetItems::DataFrame {
                data: self.data.clone(),
                order_key: vec![col("item_order")],
            }),
            measure: WidgetMeasureSpec::fixed(40.0, 24.0),
            presentation: WidgetPresentationBindings::default(),
        })
    }
}

impl ChartWidget for ExplicitPartDataWidget {
    fn id(&self) -> &str {
        "explicit-part-data"
    }

    fn kind(&self) -> &'static str {
        "explicit-part-data-widget"
    }

    fn expand(
        &self,
        _ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        Ok(WidgetExpansion {
            expansion: ToolExpansion::new().mark(
                Symbol::<PixelFrame>::new()
                    .id("dot")
                    .data(self.data.clone())
                    .x(col("x"))
                    .y(12.0)
                    .size(16.0),
            ),
            items: None,
            measure: WidgetMeasureSpec::fixed(40.0, 24.0),
            presentation: WidgetPresentationBindings::default(),
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
    assert_eq!(
        decoded.event_datum_types().get("value"),
        Some(&DataType::Utf8),
        "widget binding datum fields must be present in the compiled schema"
    );
    let evaluated = decoded.evaluate(&ctx, None).await.unwrap();
    let widget_children =
        find_group(&evaluated.scene_graph.marks, "contract").expect("compiler-owned widget group");
    let widget_group = find_scene_group(&evaluated.scene_graph.marks, "contract").unwrap();
    assert!(
        widget_group.origin[0] > 0.0,
        "right-side widget must be translated into reserved chrome"
    );
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
    let box_datums = evaluated
        .event_datums
        .rows
        .iter()
        .find(|rows| rows.mark_path == box_path)
        .unwrap_or_else(|| {
            panic!(
                "widget box event datum rows use final scene path {box_path:?}; actual paths: {:?}",
                evaluated
                    .event_datums
                    .rows
                    .iter()
                    .map(|rows| rows.mark_path.as_slice())
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(box_datums.rows.num_rows(), 2);
    assert_eq!(
        box_datums.rows.schema().field(0).name(),
        "value",
        "the widget item schema must participate in event datum inference"
    );

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
async fn one_shot_concat_child_accepts_widget_attachment() {
    let ctx = datafusion::prelude::SessionContext::new();
    let result = Chart::<HConcat>::new()
        .mark(Subplot::new(
            Plot::<Cartesian>::new().widget(ContractWidget.position(ChromePosition::Right)),
        ))
        .compile(&ctx)
        .await;

    if let Err(error) = result {
        panic!("one-shot concat child should be legal: {error}");
    }
}

#[tokio::test]
async fn facet_multiplied_child_rejects_widget_attachment() {
    let ctx = datafusion::prelude::SessionContext::new();
    let result = Chart::<FacetRow>::new()
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().widget(ContractWidget.position(ChromePosition::Right)),
            )
            .row(datafusion::prelude::col("group")),
        )
        .compile(&ctx)
        .await;

    let error = match result {
        Ok(_) => panic!("facet-multiplied widget must be rejected"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("facet- or repeat-multiplied"),
        "unexpected diagnostic: {error}"
    );
}

#[tokio::test]
async fn repeat_multiplied_child_rejects_widget_attachment() {
    let ctx = datafusion::prelude::SessionContext::new();
    let result = Chart::<RepeatColumns>::new()
        .configure_coord(|coord| {
            coord
                .columns(vec![
                    RepeatVariable::new("a", datafusion::prelude::col("a")),
                    RepeatVariable::new("b", datafusion::prelude::col("b")),
                ])
                .cell(
                    Plot::<Cartesian>::new().widget(ContractWidget.position(ChromePosition::Right)),
                )
        })
        .compile(&ctx)
        .await;

    let error = match result {
        Ok(_) => panic!("repeat-multiplied widget must be rejected"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("facet- or repeat-multiplied"),
        "unexpected diagnostic: {error}"
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
async fn guide_widgets_share_one_ordered_side_stack() {
    let ctx = datafusion::prelude::SessionContext::new();
    let compiled = Chart::<Cartesian>::new()
        .widget(
            FixedContractWidget {
                id: "first",
                height: 20.0,
            }
            .position(ChromePosition::Right),
        )
        .widget(
            FixedContractWidget {
                id: "second",
                height: 30.0,
            }
            .position(ChromePosition::Right),
        )
        .compile(&ctx)
        .await
        .unwrap();
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    let first = find_scene_group(&evaluated.scene_graph.marks, "first").unwrap();
    let second = find_scene_group(&evaluated.scene_graph.marks, "second").unwrap();
    assert_eq!(second.origin[1], first.origin[1] + 20.0);
    assert_eq!(first.origin[0], second.origin[0]);
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

#[tokio::test]
async fn widget_item_relation_bakes_and_evaluates_without_source_table() {
    let server = datafusion::prelude::SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )])),
        vec![Arc::new(Int64Array::from(vec![3, 1, 2]))],
    )
    .unwrap();
    server.register_batch("widget_source", batch).unwrap();
    let data = server.table("widget_source").await.unwrap();
    let compiled = Chart::<Cartesian>::new()
        .widget(DataFrameContractWidget { data }.position(ChromePosition::Left))
        .compile(&server)
        .await
        .unwrap();

    let (baked, report) = compiled
        .bake(&server, &BakePolicy::default())
        .await
        .unwrap();
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::Baked {
            context_id: BakeContextId::WidgetItems { id, .. },
            ..
        } if id == "data-contract"
    )));
    assert!(report.self_contained, "{:#?}", report.contexts);

    let decoded: avenger_chart::plot::CompiledPlot =
        bincode::deserialize(&bincode::serialize(&baked).unwrap()).unwrap();
    decoded
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .expect("baked widget items should evaluate without the source table");
}

#[tokio::test]
async fn widget_part_data_bakes_and_evaluates_without_source_table() {
    let server = datafusion::prelude::SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new("x", DataType::Float32, false)])),
        vec![Arc::new(datafusion::arrow::array::Float32Array::from(
            vec![8.0, 24.0],
        ))],
    )
    .unwrap();
    server.register_batch("widget_part_source", batch).unwrap();
    let data = server.table("widget_part_source").await.unwrap();
    let compiled = Chart::<Cartesian>::new()
        .widget(ExplicitPartDataWidget { data }.position(ChromePosition::Left))
        .compile(&server)
        .await
        .unwrap();

    let (baked, report) = compiled
        .bake(&server, &BakePolicy::default())
        .await
        .unwrap();
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::Baked {
            context_id: BakeContextId::WidgetPart {
                widget_id,
                part,
                ..
            },
            ..
        } if widget_id == "explicit-part-data" && part == "dot"
    )));
    assert!(report.self_contained, "{:#?}", report.contexts);

    let decoded: avenger_chart::plot::CompiledPlot =
        bincode::deserialize(&bincode::serialize(&baked).unwrap()).unwrap();
    let evaluated = decoded
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .expect("baked widget part should evaluate without the source table");
    let dot = find_symbol(
        find_group(&evaluated.scene_graph.marks, "explicit-part-data").unwrap(),
        "dot",
    )
    .unwrap();
    assert_eq!(dot.x.as_vec(2, None), vec![8.0, 24.0]);
}

#[tokio::test]
async fn widget_mark_geometry_reads_style_snapshot_and_realized_frame_inputs() {
    let ctx = datafusion::prelude::SessionContext::new();
    let theme = Theme::from_css(
        "themed-geometry-widget::part(box) { choice-control-size: 17px; fill: #cc0000; } \
         themed-geometry-widget[checked=\"true\"]::part(box) { fill: #0072b2; }",
    )
    .unwrap();
    let compiled = Chart::<Cartesian>::new()
        .theme(theme)
        .widget(ThemedGeometryWidget.position(ChromePosition::Left))
        .compile(&ctx)
        .await
        .unwrap();
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    let marks = find_group(&evaluated.scene_graph.marks, "themed-geometry").unwrap();
    let rect = marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Rect(rect) if rect.name == "box" => Some(rect),
            _ => None,
        })
        .unwrap();

    assert_eq!(rect.x2.as_ref().unwrap().as_vec(1, None), vec![17.0]);
    assert_eq!(rect.y2.as_ref().unwrap().as_vec(1, None), vec![23.0]);
    assert_eq!(
        rect.fill.as_vec(1, None)[0].color_or_transparent(),
        avenger_color::parse_color_string("#0072b2").unwrap()
    );
}

#[tokio::test]
async fn host_and_widget_visual_scales_keep_independent_domains_after_bincode() {
    let ctx = datafusion::prelude::SessionContext::new();
    let host_data = ctx
        .sql(
            "SELECT * FROM (VALUES \
             (0.0, 0.0, 'host-a', 10.0), \
             (1.0, 1.0, 'host-b', 20.0) \
             ) AS t(x, y, category, amount)",
        )
        .await
        .unwrap();
    let compiled = Chart::<Cartesian>::new()
        .data(host_data)
        .mark(
            Symbol::<Cartesian>::new()
                .id("host")
                .x(col("x"))
                .y(col("y"))
                .fill_with(col("category"), |channel| channel.no_legend())
                .size_with(col("amount"), |channel| channel.no_legend()),
        )
        .widget(
            ScaledContractWidget {
                id: "scaled-a",
                categories: ["widget-a-1", "widget-a-2"],
                amounts: [1.0, 2.0],
            }
            .position(ChromePosition::Right),
        )
        .widget(
            ScaledContractWidget {
                id: "scaled-b",
                categories: ["widget-b-1", "widget-b-2"],
                amounts: [100.0, 200.0],
            }
            .position(ChromePosition::Right),
        )
        .compile(&ctx)
        .await
        .unwrap();

    assert_eq!(compiled.legends().len(), 2);
    assert!(compiled.legends().contains_key("fill"));
    assert!(compiled.legends().contains_key("size"));
    let artifact = serde_json::to_value(&compiled).unwrap();
    assert!(artifact["widget_scale_specs"].get("scaled-a").is_some());
    assert!(artifact["widget_scale_specs"].get("scaled-b").is_some());

    let decoded = bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();
    for candidate in [compiled, decoded] {
        let evaluated = candidate.evaluate(&ctx, None).await.unwrap();
        let host = find_symbol(&evaluated.scene_graph.marks, "host").unwrap();
        let first = find_symbol(
            find_group(&evaluated.scene_graph.marks, "scaled-a").unwrap(),
            "dot",
        )
        .unwrap();
        let second = find_symbol(
            find_group(&evaluated.scene_graph.marks, "scaled-b").unwrap(),
            "dot",
        )
        .unwrap();
        let colors = |symbol: &avenger_scenegraph::marks::symbol::SceneSymbolMark| {
            symbol
                .fill_vec()
                .into_iter()
                .map(|fill| fill.color_or_transparent())
                .collect::<Vec<_>>()
        };
        assert_eq!(colors(host), colors(first));
        assert_eq!(colors(host), colors(second));
        for widget_sizes in [first.size_vec(), second.size_vec()] {
            let host_sizes = host.size_vec();
            assert_eq!(host_sizes.len(), widget_sizes.len());
            assert!(
                host_sizes
                    .iter()
                    .zip(widget_sizes)
                    .all(|(host, widget)| (host - widget).abs() < 1e-3)
            );
        }
    }
}

#[tokio::test]
async fn widget_visual_scale_domains_follow_param_driven_item_revisions() {
    let ctx = Arc::new(datafusion::prelude::SessionContext::new());
    let group = Param::new("widget_group", "low");
    let paint = Param::new("--widget-test-fill", "#0072b2");
    let all_items = ctx
        .sql(
            "SELECT * FROM (VALUES \
             (0, 'low',  'low-a',    1.0), \
             (1, 'low',  'low-b',    2.0), \
             (0, 'high', 'high-a', 100.0), \
             (1, 'high', 'high-b', 200.0) \
             ) AS t(item_order, item_group, category, amount)",
        )
        .await
        .unwrap()
        .filter(col("item_group").eq(group.expr()))
        .unwrap();
    let mut theme = Theme::light();
    theme
        .append_css("scaled-contract-widget::part(paint) { fill: var(--widget-test-fill); }")
        .unwrap();
    let compiled = Chart::<Cartesian>::new()
        .theme(theme)
        .param(group)
        .param(paint)
        .widget(DataFrameScaledWidget { data: all_items }.position(ChromePosition::Right))
        .compile(&ctx)
        .await
        .unwrap();
    let mut session = Arc::new(compiled).instantiate(ctx);

    let (initial, initial_metrics) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await
        .unwrap();
    assert_eq!(initial_metrics.pipeline.widget_item_collects, 1);
    assert_eq!(initial_metrics.pipeline.widget_item_cache_hits, 0);
    assert_eq!(initial_metrics.pipeline.widget_item_cache_misses, 1);
    let initial_symbol = find_symbol(
        find_group(&initial.scene_graph.marks, "revision-scale").unwrap(),
        "dot",
    )
    .unwrap();
    let initial_sizes = initial_symbol.size_vec();
    let initial_colors = initial_symbol
        .fill_vec()
        .into_iter()
        .map(|fill| fill.color_or_transparent())
        .collect::<Vec<_>>();
    let initial_paint = find_symbol(
        find_group(&initial.scene_graph.marks, "revision-scale").unwrap(),
        "paint",
    )
    .unwrap()
    .fill_vec()[0]
        .color_or_transparent();

    let mut paint_patch = indexmap::IndexMap::new();
    paint_patch.insert(
        "--widget-test-fill".to_string(),
        datafusion::common::ScalarValue::Utf8(Some("#d55e00".to_string())),
    );
    let (repainted, repaint_metrics) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(paint_patch))
        .await
        .unwrap();
    assert_eq!(repaint_metrics.pipeline.widget_item_collects, 0);
    assert_eq!(repaint_metrics.pipeline.widget_item_cache_hits, 1);
    assert_eq!(repaint_metrics.pipeline.widget_item_cache_misses, 0);
    let repainted_color = find_symbol(
        find_group(&repainted.scene_graph.marks, "revision-scale").unwrap(),
        "paint",
    )
    .unwrap()
    .fill_vec()[0]
        .color_or_transparent();
    assert_ne!(initial_paint, repainted_color);

    let mut patch = indexmap::IndexMap::new();
    patch.insert(
        "widget_group".to_string(),
        datafusion::common::ScalarValue::Utf8(Some("high".to_string())),
    );
    let (revised, revised_metrics) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(patch))
        .await
        .unwrap();
    assert_eq!(revised_metrics.pipeline.widget_item_collects, 1);
    assert_eq!(revised_metrics.pipeline.widget_item_cache_hits, 0);
    assert_eq!(revised_metrics.pipeline.widget_item_cache_misses, 1);
    let revised_symbol = find_symbol(
        find_group(&revised.scene_graph.marks, "revision-scale").unwrap(),
        "dot",
    )
    .unwrap();
    let revised_colors = revised_symbol
        .fill_vec()
        .into_iter()
        .map(|fill| fill.color_or_transparent())
        .collect::<Vec<_>>();

    let revised_sizes = revised_symbol.size_vec();
    assert_eq!(initial_sizes.len(), revised_sizes.len());
    assert!(
        initial_sizes
            .iter()
            .zip(revised_sizes)
            .all(|(initial, revised)| (initial - revised).abs() < 1e-3)
    );
    assert_eq!(initial_colors, revised_colors);
}
