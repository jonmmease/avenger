//! The chart-level bake census: a matrix of chart shapes, each baked with
//! the proto emit (the only emit form), serialized, deserialized into a
//! FRESH `SessionContext`, and evaluated at multiple param sets against the
//! unbaked chart evaluated over the live sources. Equivalence is asserted as
//! scene-graph BYTE equality, and every census row asserts exact per-context
//! report accounting (nothing passes through silently).

use std::sync::Arc;

use avenger_chart::{
    bake::{BakeContextId, BakePolicy, ContextBakeStatus, NotBakedReason, PlotBakeReport},
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

fn segmented_sales_batch() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("segment", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["EU", "EU", "EU", "NA", "NA", "NA"])),
            Arc::new(StringArray::from(vec![
                "core", "core", "edge", "core", "edge", "edge",
            ])),
            Arc::new(Float64Array::from(vec![1.0, 2.0, 4.0, 8.0, 16.0, 32.0])),
        ],
    )
    .expect("segmented sales batch")
}

fn params(pairs: &[(&str, f64)]) -> IndexMap<String, ScalarValue> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), ScalarValue::Float64(Some(*value))))
        .collect()
}

type ParamPair = (IndexMap<String, ScalarValue>, IndexMap<String, ScalarValue>);

/// Bake, round-trip through bincode, and assert scene-graph BYTE equality
/// between the unbaked chart (live sources, `unbaked_params`) and the baked
/// chart (fresh session, `baked_params`) at every param set.
async fn assert_bake_equivalence(
    label: &str,
    server_ctx: &SessionContext,
    compiled: &CompiledPlot,
    policy: &BakePolicy,
    param_sets: &[ParamPair],
) -> Result<PlotBakeReport, Box<dyn std::error::Error>> {
    let (baked, report) = compiled.bake(server_ctx, policy).await?;
    let encoded = bincode::serialize(&baked)?;
    let decoded: CompiledPlot = bincode::deserialize(&encoded)?;
    let client_ctx = SessionContext::new();

    assert!(!param_sets.is_empty());
    for (unbaked_params, baked_params) in param_sets {
        let unbaked_eval = compiled
            .evaluate(server_ctx, Some(unbaked_params.clone()))
            .await?;
        let baked_eval = decoded
            .evaluate(&client_ctx, Some(baked_params.clone()))
            .await?;
        assert!(baked_eval.scene_graph.width > 0.0);
        assert_eq!(
            bincode::serialize(&baked_eval.scene_graph)?,
            bincode::serialize(&unbaked_eval.scene_graph)?,
            "census `{label}`: baked and unbaked scenes diverge at params {unbaked_params:?}"
        );
    }
    Ok(report)
}

fn count_baked(report: &PlotBakeReport) -> usize {
    report
        .contexts
        .iter()
        .filter(|status| matches!(status, ContextBakeStatus::Baked { .. }))
        .count()
}

/// Unfaceted, plot-level data with a live `$min` filter above a folded
/// aggregate.
#[tokio::test]
async fn census_unfaceted_param_filter() -> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    server_ctx.register_batch("sales", sales_batch())?;
    let data = server_ctx
        .sql(
            "SELECT * FROM (SELECT region, SUM(value) AS total \
             FROM sales GROUP BY region) q WHERE total > $min ORDER BY region",
        )
        .await?;
    let compiled = Chart::<Cartesian>::new()
        .data(data)
        .mark(Symbol::new().x(col("total")).y(col("total")).size(64.0))
        .compile(&server_ctx)
        .await?;

    let sets = [2.5, 4.5]
        .map(|min| (params(&[("min", min)]), params(&[("min", min)])))
        .to_vec();
    let report = assert_bake_equivalence(
        "unfaceted",
        &server_ctx,
        &compiled,
        &BakePolicy::default(),
        &sets,
    )
    .await?;

    assert_eq!(report.contexts.len(), 1, "{:#?}", report.contexts);
    assert_eq!(count_baked(&report), 1);
    assert!(report.self_contained);
    Ok(())
}

/// Faceted: the plot data (with a live param filter) bakes to one table
/// serving all cells; the per-cell aggregate chain stays live.
#[tokio::test]
async fn census_faceted_aggregate() -> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    server_ctx.register_batch("segmented_sales", segmented_sales_batch())?;
    let data = server_ctx
        .sql("SELECT * FROM segmented_sales WHERE value > $min")
        .await?;
    let leaf = Plot::<Cartesian>::new().mark(MarkGroup::new().transform(
        Aggregate::new().sum("total", col("value")),
        |group, aggregate| {
            group.mark(
                Symbol::new()
                    .x(aggregate.output("total"))
                    .y(aggregate.output("total"))
                    .size(72.0),
            )
        },
    ));
    let compiled = Chart::<FacetColumn>::new()
        .canvas_size(520.0, 260.0)
        .data(data)
        .mark(Subplot::new(leaf).column(col("region")))
        .compile(&server_ctx)
        .await?;

    let sets = [0.5, 3.5]
        .map(|min| (params(&[("min", min)]), params(&[("min", min)])))
        .to_vec();
    let report = assert_bake_equivalence(
        "faceted",
        &server_ctx,
        &compiled,
        &BakePolicy::default(),
        &sets,
    )
    .await?;

    // Root plot data baked; child plot inherits (NoData); the per-cell chain
    // stays live.
    assert_eq!(report.contexts.len(), 3, "{:#?}", report.contexts);
    assert_eq!(count_baked(&report), 1);
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::NotBaked {
            context_id: BakeContextId::ChildMarkGroup { .. },
            reason: NotBakedReason::FacetScopedTransforms,
        }
    )));
    assert!(report.self_contained, "{:#?}", report.contexts);
    Ok(())
}

/// Facet wrap: like the faceted row, the plot data (with a live param
/// filter) bakes to one table serving all cells and the per-cell chain
/// stays live — but wrap marks RENDER through a regenerated physical
/// lowering, so byte-equality here proves the physical-subplot
/// regeneration keeps payload and render path consistent. The
/// param-filtered plot data is the shape that exposed the facet_col guide
/// fail-open.
#[tokio::test]
async fn census_facet_wrap() -> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    server_ctx.register_batch("segmented_sales", segmented_sales_batch())?;
    let data = server_ctx
        .sql("SELECT * FROM segmented_sales WHERE value > $min")
        .await?;
    let leaf = Plot::<Cartesian>::new().mark(MarkGroup::new().transform(
        Aggregate::new().sum("total", col("value")),
        |group, aggregate| {
            group.mark(
                Symbol::new()
                    .x(aggregate.output("total"))
                    .y(aggregate.output("total"))
                    .size(72.0),
            )
        },
    ));
    let compiled = Chart::<FacetWrap>::new()
        .canvas_size(520.0, 260.0)
        .data(data)
        .mark(Subplot::new(leaf).wrap_with(col("region"), |c| c.columns(2)))
        .compile(&server_ctx)
        .await?;

    let sets = [0.5, 3.5]
        .map(|min| (params(&[("min", min)]), params(&[("min", min)])))
        .to_vec();
    let report = assert_bake_equivalence(
        "facet-wrap",
        &server_ctx,
        &compiled,
        &BakePolicy::default(),
        &sets,
    )
    .await?;

    // Root plot data baked; child plot inherits (NoData); the per-cell
    // chain stays live.
    assert_eq!(report.contexts.len(), 3, "{:#?}", report.contexts);
    assert_eq!(count_baked(&report), 1);
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::NotBaked {
            context_id: BakeContextId::ChildMarkGroup { .. },
            reason: NotBakedReason::FacetScopedTransforms,
        }
    )));
    assert!(report.self_contained, "{:#?}", report.contexts);
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

/// Store-bearing: the store group stays live and materializes identically on
/// both sides; the static plot data bakes.
#[tokio::test]
async fn census_store_bearing() -> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    server_ctx.register_batch("sales", sales_batch())?;
    let data = server_ctx
        .sql(
            "SELECT * FROM (SELECT region, SUM(value) AS total \
             FROM sales GROUP BY region) q WHERE total > $min ORDER BY region",
        )
        .await?;
    let compiled = Chart::<Cartesian>::new()
        .data(data)
        .store(
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
        .await?;

    let sets = [2.5, 4.5]
        .map(|min| (params(&[("min", min)]), params(&[("min", min)])))
        .to_vec();
    let report = assert_bake_equivalence(
        "store-bearing",
        &server_ctx,
        &compiled,
        &BakePolicy::default(),
        &sets,
    )
    .await?;

    // Plot data + symbol group (NoData, inherits) + store group (live).
    assert_eq!(report.contexts.len(), 3, "{:#?}", report.contexts);
    assert_eq!(count_baked(&report), 1);
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::NotBaked {
            reason: NotBakedReason::StoreData,
            ..
        }
    )));
    Ok(())
}

/// Fixed params: `$max` is bound at bake time (the baked chart no longer
/// takes it), `$min` stays live on both sides.
#[tokio::test]
async fn census_fixed_params() -> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    server_ctx.register_batch("sales", sales_batch())?;
    let data = server_ctx
        .sql(
            "SELECT * FROM (SELECT region, SUM(value) AS total \
             FROM sales GROUP BY region) q \
             WHERE total > $min AND total < $max ORDER BY region",
        )
        .await?;
    let compiled = Chart::<Cartesian>::new()
        .data(data)
        .mark(Symbol::new().x(col("total")).y(col("total")).size(64.0))
        .compile(&server_ctx)
        .await?;

    let policy = BakePolicy {
        fixed_params: vec![("max".to_string(), ScalarValue::Float64(Some(8.0)))],
        ..BakePolicy::default()
    };
    // Unbaked binds BOTH params; the baked chart binds only the live one.
    let sets = [2.5, 4.5]
        .map(|min| {
            (
                params(&[("min", min), ("max", 8.0)]),
                params(&[("min", min)]),
            )
        })
        .to_vec();
    let report =
        assert_bake_equivalence("fixed-params", &server_ctx, &compiled, &policy, &sets).await?;

    assert_eq!(report.contexts.len(), 1);
    assert_eq!(count_baked(&report), 1);
    assert_eq!(
        report
            .fixed_params_applied
            .iter()
            .map(|binding| binding.name.as_str())
            .collect::<Vec<_>>(),
        vec!["max"]
    );
    assert_eq!(report.remaining_params, vec!["$min".to_string()]);
    Ok(())
}

/// Multi-mark shared chain: two groups over the same param-filtered query
/// dedup to ONE baked table (shared primary).
#[tokio::test]
async fn census_multi_mark_shared_chain() -> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    server_ctx.register_batch("sales", sales_batch())?;
    let sql = "SELECT * FROM (SELECT region, SUM(value) AS total \
               FROM sales GROUP BY region) q WHERE total > $min ORDER BY region";
    let left = server_ctx.sql(sql).await?;
    let right = server_ctx.sql(sql).await?;
    let compiled = Chart::<Cartesian>::new()
        .mark(
            MarkGroup::new()
                .data(left)
                .mark(Symbol::new().x(col("total")).y(col("total")).size(48.0)),
        )
        .mark(
            MarkGroup::new()
                .data(right)
                .mark(Symbol::new().x(col("total")).y(col("total")).size(24.0)),
        )
        .compile(&server_ctx)
        .await?;

    let sets = [2.5, 4.5]
        .map(|min| (params(&[("min", min)]), params(&[("min", min)])))
        .to_vec();
    let report = assert_bake_equivalence(
        "shared-chain",
        &server_ctx,
        &compiled,
        &BakePolicy::default(),
        &sets,
    )
    .await?;

    // Plot data (none → NoData) + two baked groups sharing one primary.
    assert_eq!(report.contexts.len(), 3, "{:#?}", report.contexts);
    assert_eq!(count_baked(&report), 2);
    let primaries = report
        .contexts
        .iter()
        .filter_map(|status| match status {
            ContextBakeStatus::Baked { primary_table, .. } => Some(primary_table.clone()),
            ContextBakeStatus::NotBaked { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(primaries[0], primaries[1]);
    assert!(report.self_contained);
    Ok(())
}

/// Sql-authored: a user `sql` stage over inherited plot data folds through
/// the full-chain bake with its `$min` placeholder live.
#[tokio::test]
async fn census_sql_authored() -> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    server_ctx.register_batch("sales", sales_batch())?;
    let data = server_ctx.table("sales").await?;
    let compiled = Chart::<Cartesian>::new()
        .data(data)
        .mark(MarkGroup::new().transform(
            Sql::new(
                "SELECT region, SUM(value) AS total FROM input \
                 WHERE value >= $min GROUP BY region ORDER BY region",
            ),
            |group, _| group.mark(Symbol::new().x(col("total")).y(col("total")).size(64.0)),
        ))
        .compile(&server_ctx)
        .await?;

    let sets = [0.5, 2.5]
        .map(|min| (params(&[("min", min)]), params(&[("min", min)])))
        .to_vec();
    let report = assert_bake_equivalence(
        "sql-authored",
        &server_ctx,
        &compiled,
        &BakePolicy::default(),
        &sets,
    )
    .await?;

    // Plot data + the sql-chain group, both baked.
    assert_eq!(report.contexts.len(), 2, "{:#?}", report.contexts);
    assert_eq!(count_baked(&report), 2);
    assert!(report.self_contained);
    Ok(())
}
