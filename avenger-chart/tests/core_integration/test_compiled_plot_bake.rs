use std::sync::Arc;

use avenger_chart::{
    bake::{BakeContextId, BakePolicy, ContextBakeStatus, FixedParamBinding, NotBakedReason},
    plot::CompiledPlot,
    prelude::*,
};
use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};
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

fn tiny_sales_batch() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["EU"])),
            Arc::new(Float64Array::from(vec![1.0])),
        ],
    )
    .expect("tiny sales batch")
}

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

fn aggregate_leaf_plot() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(MarkGroup::new().transform(
        Aggregate::new().sum("total", col("value")),
        |group, aggregate| {
            group.mark(
                Symbol::new()
                    .x(aggregate.output("total"))
                    .y(aggregate.output("total"))
                    .size(72.0),
            )
        },
    ))
}

async fn segmented_sales_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    ctx.register_batch("segmented_sales", segmented_sales_batch())
        .expect("register segmented sales");
    ctx.table("segmented_sales")
        .await
        .expect("segmented sales table")
}

async fn compiled_faceted_aggregate_plot(ctx: &SessionContext) -> CompiledPlot {
    let data = segmented_sales_dataframe(ctx).await;
    Plot::<FacetColumn>::new()
        .canvas_size(520.0, 260.0)
        .data(data)
        .mark(Subplot::new(aggregate_leaf_plot()).column(col("region")))
        .compile(ctx)
        .await
        .expect("compile faceted aggregate plot")
}

async fn compiled_nested_faceted_aggregate_plot(ctx: &SessionContext) -> CompiledPlot {
    let data = segmented_sales_dataframe(ctx).await;
    let inner =
        Plot::<FacetColumn>::new().mark(Subplot::new(aggregate_leaf_plot()).column(col("segment")));
    Plot::<FacetRow>::new()
        .canvas_size(560.0, 360.0)
        .data(data)
        .mark(Subplot::new(inner).row(col("region")))
        .compile(ctx)
        .await
        .expect("compile nested faceted aggregate plot")
}

fn params(min: f64) -> IndexMap<String, ScalarValue> {
    IndexMap::from([("min".to_string(), ScalarValue::Float64(Some(min)))])
}

fn symbol_positions(scene: &SceneGraph) -> Vec<(i32, i32)> {
    fn walk(mark: &SceneMark, positions: &mut Vec<(i32, i32)>) {
        match mark {
            SceneMark::Group(group) => {
                for child in &group.marks {
                    walk(child, positions);
                }
            }
            SceneMark::Symbol(symbol) => {
                let len = symbol.len as usize;
                let xs = symbol.x.as_vec(len, None);
                let ys = symbol.y.as_vec(len, None);
                positions.extend(
                    xs.into_iter()
                        .zip(ys)
                        .map(|(x, y)| ((x * 1000.0).round() as i32, (y * 1000.0).round() as i32)),
                );
            }
            _ => {}
        }
    }

    let mut positions = Vec::new();
    for mark in &scene.marks {
        walk(mark, &mut positions);
    }
    positions.sort_unstable();
    positions
}

#[tokio::test]
async fn baked_plot_evaluates_in_fresh_session_without_sources()
-> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    let compiled = compiled_sales_plot(&server_ctx).await;

    let (baked, report) = compiled.bake(&server_ctx, &BakePolicy::default()).await?;
    assert!(report.self_contained, "{:#?}", report.contexts);
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

/// A faceted chart bakes through its plot data: ONE baked table (raw rows
/// with facet-key columns) serves every cell, the per-cell aggregate chain
/// stays live on top of it, and the baked plot renders identically to the
/// unbaked plot in a fresh session with no source tables.
#[tokio::test]
async fn faceted_aggregate_bakes_plot_data_serving_all_cells()
-> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    let compiled = compiled_faceted_aggregate_plot(&server_ctx).await;

    let (baked, report) = compiled.bake(&server_ctx, &BakePolicy::default()).await?;
    assert!(report.self_contained, "{:#?}", report.contexts);
    let primary_tables = report
        .contexts
        .iter()
        .filter_map(|status| match status {
            ContextBakeStatus::Baked {
                context_id,
                primary_table,
                ..
            } => Some((context_id.clone(), primary_table.clone())),
            ContextBakeStatus::NotBaked { .. } => None,
        })
        .collect::<Vec<_>>();
    // Exactly one baked context — the root plot data — whose single table
    // serves every cell.
    assert_eq!(primary_tables.len(), 1, "{:#?}", report.contexts);
    assert!(matches!(primary_tables[0].0, BakeContextId::PlotData));
    // The inherited per-cell chain must stay live: shared-scale domains
    // evaluate it at the sharing-owner scope, which no pre-grouped table can
    // reproduce.
    assert!(
        report.contexts.iter().any(|status| matches!(
            status,
            ContextBakeStatus::NotBaked {
                context_id: BakeContextId::ChildMarkGroup {
                    subplot_path,
                    index: 0,
                    ..
                },
                reason: NotBakedReason::FacetScopedTransforms,
            } if subplot_path == &[0]
        )),
        "{:#?}",
        report.contexts
    );

    let encoded = bincode::serialize(&baked)?;
    let decoded: CompiledPlot = bincode::deserialize(&encoded)?;
    let client_ctx = SessionContext::new();
    let unbaked_eval = compiled.evaluate(&server_ctx, None).await?;
    let baked_eval = decoded.evaluate(&client_ctx, None).await?;
    assert_eq!(
        symbol_positions(&baked_eval.scene_graph),
        symbol_positions(&unbaked_eval.scene_graph),
        "faceted baked and unbaked symbol geometry diverges"
    );
    Ok(())
}

#[tokio::test]
async fn nested_faceted_aggregate_bakes_root_data_and_keeps_leaf_chain_live()
-> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    let compiled = compiled_nested_faceted_aggregate_plot(&server_ctx).await;

    let (baked, report) = compiled.bake(&server_ctx, &BakePolicy::default()).await?;
    assert!(report.self_contained, "{:#?}", report.contexts);
    assert!(
        report.contexts.iter().any(|status| matches!(
            status,
            ContextBakeStatus::NotBaked {
                context_id: BakeContextId::ChildMarkGroup {
                    subplot_path,
                    index: 0,
                    ..
                },
                reason: NotBakedReason::FacetScopedTransforms,
            } if subplot_path == &[0, 0]
        )),
        "{:#?}",
        report.contexts
    );

    let encoded = bincode::serialize(&baked)?;
    let decoded: CompiledPlot = bincode::deserialize(&encoded)?;
    let client_ctx = SessionContext::new();
    let unbaked_eval = compiled.evaluate(&server_ctx, None).await?;
    let baked_eval = decoded.evaluate(&client_ctx, None).await?;
    assert_eq!(
        symbol_positions(&baked_eval.scene_graph),
        symbol_positions(&unbaked_eval.scene_graph),
        "nested faceted baked and unbaked symbol geometry diverges"
    );
    Ok(())
}

/// An unfaceted mark group that inherits plot data through a plan-pure chain
/// bakes to the chain's output (the chain no longer re-runs per evaluation).
#[tokio::test]
async fn unfaceted_inherited_chain_bakes_to_chain_output() -> Result<(), Box<dyn std::error::Error>>
{
    let server_ctx = SessionContext::new();
    server_ctx
        .register_batch("sales", sales_batch())
        .expect("register sales");
    let data = server_ctx.table("sales").await?;
    let compiled = Plot::<Cartesian>::new()
        .canvas_size(320.0, 240.0)
        .data(data)
        .mark(
            MarkGroup::new().transform(
                Aggregate::new()
                    .group_by([col("region")])
                    .sum("total", col("value")),
                |group, aggregate| {
                    group.mark(
                        Symbol::new()
                            .x(aggregate.output("total"))
                            .y(aggregate.output("total"))
                            .size(48.0),
                    )
                },
            ),
        )
        .compile(&server_ctx)
        .await?;

    let (baked, report) = compiled.bake(&server_ctx, &BakePolicy::default()).await?;
    assert!(report.self_contained, "{:#?}", report.contexts);
    assert!(
        report.contexts.iter().any(|status| matches!(
            status,
            ContextBakeStatus::Baked {
                context_id: BakeContextId::MarkGroup { index: 0, .. },
                ..
            }
        )),
        "{:#?}",
        report.contexts
    );

    let encoded = bincode::serialize(&baked)?;
    let decoded: CompiledPlot = bincode::deserialize(&encoded)?;
    let client_ctx = SessionContext::new();
    let unbaked_eval = compiled.evaluate(&server_ctx, None).await?;
    let baked_eval = decoded.evaluate(&client_ctx, None).await?;
    assert_eq!(
        symbol_positions(&baked_eval.scene_graph),
        symbol_positions(&unbaked_eval.scene_graph),
        "unfaceted inherited-chain baked and unbaked symbol geometry diverges"
    );
    Ok(())
}

/// A mark group with its OWN explicit data inside a faceted plot gets a
/// base-only bake: the explicit base folds into a baked table while the
/// transform chain stays live on top of it (a wrong full-chain emit would
/// drop the aggregate and place symbols at raw values, failing the geometry
/// equality below).
#[tokio::test]
async fn faceted_explicit_group_bakes_base_and_keeps_chain_live()
-> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    server_ctx
        .register_batch("sales", sales_batch())
        .expect("register sales");
    let facet_data = segmented_sales_dataframe(&server_ctx).await;
    let group_data = server_ctx.table("sales").await?;
    let leaf = Plot::<Cartesian>::new().mark(
        MarkGroup::new().data(group_data).transform(
            Aggregate::new()
                .group_by([col("region")])
                .sum("total", col("value")),
            |group, aggregate| {
                group.mark(
                    Symbol::new()
                        .x(aggregate.output("total"))
                        .y(aggregate.output("total"))
                        .size(48.0),
                )
            },
        ),
    );
    let compiled = Plot::<FacetColumn>::new()
        .canvas_size(520.0, 260.0)
        .data(facet_data)
        .mark(Subplot::new(leaf).column(col("region")))
        .compile(&server_ctx)
        .await?;

    let (baked, report) = compiled.bake(&server_ctx, &BakePolicy::default()).await?;
    assert!(report.self_contained, "{:#?}", report.contexts);
    assert!(
        report.contexts.iter().any(|status| matches!(
            status,
            ContextBakeStatus::Baked {
                context_id: BakeContextId::ChildMarkGroup {
                    subplot_path,
                    index: 0,
                    ..
                },
                self_contained: true,
                ..
            } if subplot_path == &[0]
        )),
        "{:#?}",
        report.contexts
    );

    let encoded = bincode::serialize(&baked)?;
    let decoded: CompiledPlot = bincode::deserialize(&encoded)?;
    let client_ctx = SessionContext::new();
    let unbaked_eval = compiled.evaluate(&server_ctx, None).await?;
    let baked_eval = decoded.evaluate(&client_ctx, None).await?;
    assert_eq!(
        symbol_positions(&baked_eval.scene_graph),
        symbol_positions(&unbaked_eval.scene_graph),
        "faceted explicit-group baked and unbaked symbol geometry diverges"
    );
    Ok(())
}

/// A live (facet-skipped) chain that reads a session side table from inside
/// a `sql` stage keeps the chart dependent on that table, so the plot-wide
/// self-containment flag must be false even though the plot data itself
/// baked cleanly.
#[tokio::test]
async fn live_sql_side_table_flips_self_containment() -> Result<(), Box<dyn std::error::Error>> {
    let server_ctx = SessionContext::new();
    server_ctx
        .register_batch("side_thresholds", sales_batch())
        .expect("register side thresholds");
    let data = segmented_sales_dataframe(&server_ctx).await;
    let leaf = Plot::<Cartesian>::new().mark(MarkGroup::new().transform(
        Sql::new(
            "SELECT input.value, s.value AS threshold FROM input \
             JOIN side_thresholds s ON input.region = s.region",
        ),
        |group, _| group.mark(Symbol::new().x(col("value")).y(col("threshold")).size(48.0)),
    ));
    let compiled = Plot::<FacetColumn>::new()
        .canvas_size(520.0, 260.0)
        .data(data)
        .mark(Subplot::new(leaf).column(col("region")))
        .compile(&server_ctx)
        .await?;

    let (_baked, report) = compiled.bake(&server_ctx, &BakePolicy::default()).await?;

    // The plot data still bakes...
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::Baked {
            context_id: BakeContextId::PlotData,
            ..
        }
    )));
    // ...the per-cell chain stays live...
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::NotBaked {
            reason: NotBakedReason::FacetScopedTransforms,
            ..
        }
    )));
    // ...and the live chain's side-table read makes the plot NOT
    // self-contained.
    assert!(!report.self_contained, "{:#?}", report.contexts);
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

fn large_threshold_batch() -> RecordBatch {
    let regions = (0..4096)
        .map(|index| if index % 2 == 0 { "EU" } else { "NA" })
        .collect::<Vec<_>>();
    let thresholds = (0..4096).map(|index| index as f64).collect::<Vec<_>>();
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("threshold", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(regions)),
            Arc::new(Float64Array::from(thresholds)),
        ],
    )
    .expect("large threshold batch")
}

#[tokio::test]
async fn non_self_contained_bake_is_flagged_not_errored() -> Result<(), Box<dyn std::error::Error>>
{
    let server_ctx = SessionContext::new();
    server_ctx
        .register_batch("small_sales", tiny_sales_batch())
        .expect("register small sales");
    server_ctx
        .register_batch("live_thresholds", large_threshold_batch())
        .expect("register live thresholds");

    let data = server_ctx
        .table("small_sales")
        .await
        .expect("small sales table");
    let compiled = Plot::<Cartesian>::new()
        .mark(MarkGroup::new().data(data).transform(
            Sql::new(
                "SELECT input.region, input.value \
                         FROM input \
                         JOIN live_thresholds \
                           ON input.region = live_thresholds.region \
                         WHERE live_thresholds.threshold > $min",
            ),
            |group, _| group.mark(Symbol::new().x(col("value")).y(col("value")).size(64.0)),
        ))
        .compile(&server_ctx)
        .await
        .expect("compile side-table plot");

    let policy = BakePolicy {
        max_baked_bytes_per_subtree: 1024,
        max_baked_bytes_total: 16 * 1024,
        ..BakePolicy::default()
    };
    let (_baked, report) = compiled.bake(&server_ctx, &policy).await?;

    assert!(!report.self_contained);
    assert!(report.contexts.iter().any(|status| matches!(
        status,
        ContextBakeStatus::Baked {
            context_id: BakeContextId::MarkGroup { .. },
            self_contained: false,
            ..
        }
    )));
    Ok(())
}
