use std::sync::Arc;

use avenger_chart::{
    bake::{BakeContextId, BakePolicy, ContextBakeStatus, FixedParamBinding, NotBakedReason},
    plot::CompiledPlot,
    prelude::*,
};
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    prelude::{SessionContext, col, lit},
};
use indexmap::IndexMap;

fn sales_batch() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["EU", "EU", "NA", "NA", "APAC"])),
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])),
        ],
    )
    .expect("sales batch")
}

async fn compiled_sales_plot(ctx: &SessionContext) -> CompiledPlot {
    ctx.register_batch("sales", sales_batch())
        .expect("register sales");
    // ORDER BY keeps row order deterministic so baked and unbaked
    // evaluations can be compared byte-for-byte.
    let data = ctx
        .sql(
            "SELECT * FROM (SELECT region, SUM(value) AS total \
             FROM sales GROUP BY region) q WHERE total > $min ORDER BY region",
        )
        .await
        .expect("sales query");

    Plot::<Cartesian>::new()
        .data(data)
        .mark(Symbol::new().x(col("total")).y(col("total")).size(64.0))
        .compile(ctx)
        .await
        .expect("compile plot")
}

fn params(min: f64) -> IndexMap<String, ScalarValue> {
    IndexMap::from([("min".to_string(), ScalarValue::Float64(Some(min)))])
}

#[tokio::test]
async fn baked_plot_evaluates_in_fresh_session_without_sources()
-> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    let compiled = compiled_sales_plot(&server_ctx).await;

    let (baked, report) = compiled.bake(&server_ctx, &BakePolicy::default()).await?;
    assert!(report.self_contained);
    assert_eq!(report.source_tables, vec!["sales".to_string()]);
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::Baked {
            context_id: BakeContextId::PlotData,
            ..
        }
    )));
    // Exact accounting: a plain mark creates no mark group, so the plot data
    // context is the only entry — nothing passes through silently.
    assert_eq!(report.contexts.len(), 1);

    let encoded = bincode::serialize(&baked)?;
    let decoded: CompiledPlot = bincode::deserialize(&encoded)?;
    let client_ctx = SessionContext::new();

    // Equivalence at multiple param values: the baked plot, evaluated in a
    // fresh session with no tables registered, must produce a scene graph
    // identical to the unbaked plot evaluated against the live sources.
    for min in [2.5, 4.5] {
        let unbaked_eval = compiled.evaluate(&server_ctx, Some(params(min))).await?;
        let baked_eval = decoded.evaluate(&client_ctx, Some(params(min))).await?;
        assert!(baked_eval.scene_graph.width > 0.0);
        assert!(baked_eval.scene_graph.height > 0.0);
        assert_eq!(
            bincode::serialize(&baked_eval.scene_graph)?,
            bincode::serialize(&unbaked_eval.scene_graph)?,
            "baked and unbaked scene graphs diverge at min={min}"
        );
    }
    Ok(())
}

fn threshold_store_batch() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("lo", DataType::Float64, false),
            Field::new("hi", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["active"])),
            Arc::new(Float64Array::from(vec![3.0])),
            Arc::new(Float64Array::from(vec![6.0])),
        ],
    )
    .expect("threshold store batch")
}

#[tokio::test]
async fn store_backed_context_is_excluded_from_bake() -> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    server_ctx
        .register_batch("sales", sales_batch())
        .expect("register sales");
    let data = server_ctx
        .sql(
            "SELECT * FROM (SELECT region, SUM(value) AS total \
             FROM sales GROUP BY region) q WHERE total > $min ORDER BY region",
        )
        .await
        .expect("sales query");

    let compiled = Plot::<Cartesian>::new()
        .data(data)
        .add_store(
            Store::from_record_batch("threshold_band", threshold_store_batch())
                .primary_key(["id"])
                .sharing(CoordinationScope::Shared),
        )
        .mark(MarkGroup::new().mark(Symbol::new().x(col("total")).y(col("total")).size(64.0)))
        .mark(
            MarkGroup::new()
                .data_store(StoreData::new("threshold_band"))
                .mark(
                    Rect::new()
                        .x(lit(2.0))
                        .x2(lit(4.0))
                        .y(col("lo"))
                        .y2(col("hi"))
                        .fill("rgba(37, 99, 235, 0.18)"),
                ),
        )
        .compile(&server_ctx)
        .await
        .expect("compile store-backed plot");

    let (baked, report) = compiled.bake(&server_ctx, &BakePolicy::default()).await?;

    // Exact accounting: plot data + two mark groups, no silent pass-through.
    assert_eq!(report.contexts.len(), 3);
    // The inherit-mode group has no plan of its own and reports NoData.
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::NotBaked {
            context_id: BakeContextId::MarkGroup { .. },
            reason: NotBakedReason::NoData,
        }
    )));

    // The store-backed mark group must stay live, not freeze into a bake.
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::NotBaked {
            context_id: BakeContextId::MarkGroup { .. },
            reason: NotBakedReason::StoreData,
        }
    )));
    // Only the plot's static source folded; nothing store-related was
    // consumed by the partial evaluator.
    assert_eq!(report.source_tables, vec!["sales".to_string()]);
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::Baked {
            context_id: BakeContextId::PlotData,
            ..
        }
    )));

    // The baked plot still evaluates in a fresh session: baked data serves
    // the plot context while the store materializes live from its spec.
    let encoded = bincode::serialize(&baked)?;
    let decoded: CompiledPlot = bincode::deserialize(&encoded)?;
    let client_ctx = SessionContext::new();
    for min in [2.5, 4.5] {
        let evaluated = decoded.evaluate(&client_ctx, Some(params(min))).await?;
        assert!(evaluated.scene_graph.width > 0.0);
        assert!(evaluated.scene_graph.height > 0.0);
    }
    Ok(())
}

#[tokio::test]
async fn fixed_params_are_applied_to_baked_plot_report() -> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    let compiled = compiled_sales_plot(&server_ctx).await;
    let policy = BakePolicy {
        fixed_params: vec![("min".to_string(), ScalarValue::Float64(Some(4.5)))],
        ..BakePolicy::default()
    };

    let (baked, report) = compiled.bake(&server_ctx, &policy).await?;
    assert!(report.remaining_params.is_empty());
    assert_eq!(
        report.fixed_params_applied,
        vec![FixedParamBinding {
            name: "min".to_string(),
            value: "Float64(4.5)".to_string(),
        }]
    );

    let encoded = bincode::serialize(&baked)?;
    let decoded: CompiledPlot = bincode::deserialize(&encoded)?;
    let evaluated = decoded.evaluate(&SessionContext::new(), None).await?;
    assert!(evaluated.scene_graph.width > 0.0);
    Ok(())
}
