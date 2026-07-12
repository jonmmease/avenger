use std::sync::Arc;

use avenger_chart::{
    pixel_frame::{PixelFrameRectPositionChannels, PixelFrameTextPositionChannels},
    prelude::*,
};
use datafusion::arrow::{
    array::{Int64Array, RecordBatch},
    datatypes::{DataType, Field, Schema},
};

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
                ),
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
                            property: WidgetStyleProperty::PaddingInline,
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
    for runtime_only in ["resolved_style", "presentation_state", "theme"] {
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
    assert_eq!(widget.marks.len(), 2);
    assert_eq!(widget.relative_target_paths["contract.box"], vec![vec![0]]);
    assert_eq!(
        widget.relative_target_paths["contract.label"],
        vec![vec![1]]
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
