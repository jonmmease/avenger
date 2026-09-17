mod common;
use avenger_datafusion_dataflow::{
    arrow::datatypes::DataType,
    datafusion::{
        common::ScalarValue,
        execution::context::SessionContext,
        logical_expr::{col, lit, LogicalPlanBuilder},
    },
    CacheConfig, CachePolicy, DataflowBuilder, Result, Runtime, RuntimeConfig,
};

#[tokio::test]
async fn parameter_versions_namespaces_and_snapshot_lifetime() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let input = b.table_input("input", common::schema())?;
    let cutoff = b.scalar_input("cutoff", DataType::Int64)?;
    let irrelevant = b.scalar_input("irrelevant", DataType::Int64)?;
    let filtered = b.add_plan(
        "filtered",
        LogicalPlanBuilder::from(input.plan_ref())
            .filter(col("value").gt(cutoff.expr_ref()))?
            .build()?,
    )?;
    let out = b.table_output("rows", &filtered)?;
    let graph = b.finish()?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let p = runtime.prepare(&graph).await?;
    let values = common::snapshot(&[1, 2, 3]);
    let inputs = p
        .inputs()
        .table(&input, values.clone())?
        .scalar(&cutoff, 1_i64.into())?
        .scalar(&irrelevant, 0_i64.into())?
        .finish()?;
    assert_eq!(
        common::values(p.query(&[out], &[], &inputs).await?.table(&out)?),
        [2, 3]
    );
    let changed = inputs.edit().scalar(&cutoff, 2_i64.into())?.finish()?;
    assert_eq!(
        common::values(p.query(&[out], &[], &changed).await?.table(&out)?),
        [3]
    );
    let a_again = inputs.edit().scalar(&irrelevant, 99_i64.into())?.finish()?;
    let hit = p.clone().query(&[out], &[], &a_again).await?;
    assert_eq!(hit.report().cache_hits, 1);
    assert_eq!(hit.report().physical_plans, 0);
    let replacement = inputs
        .edit()
        .table(&input, common::snapshot(&[7]))?
        .finish()?;
    assert_eq!(
        common::values(p.query(&[out], &[], &replacement).await?.table(&out)?),
        [7]
    );
    let separate = runtime.prepare(&graph).await?;
    assert_eq!(
        separate
            .query(&[out], &[], &inputs)
            .await?
            .report()
            .cache_misses,
        1
    );
    separate.clear_results();
    assert_eq!(p.query(&[out], &[], &inputs).await?.report().cache_hits, 1);
    drop(separate);
    p.clear_results();
    assert_eq!(runtime.cache_stats().entries, 0);
    assert_eq!(common::values(hit.table(&out)?), [2, 3]);
    Ok(())
}

#[tokio::test]
async fn cached_child_skips_evicted_source_and_sources_reuse_across_filters() -> Result<()> {
    let context = SessionContext::new();
    let table = common::snapshot(&[1, 2, 3]);
    let source_plan = context
        .read_batch(table.batches()[0].clone())?
        .into_unoptimized_plan();
    let mut b = DataflowBuilder::new();
    let source = b.add_plan("source", source_plan)?;
    let cutoff = b.scalar_input("cutoff", DataType::Int64)?;
    let filtered = b.add_plan(
        "filtered",
        LogicalPlanBuilder::from(source.plan_ref())
            .filter(col("value").gt(cutoff.expr_ref()))?
            .build()?,
    )?;
    let out = b.table_output("rows", &filtered)?;
    let graph = b.finish()?;
    for max_entries in [1, 10] {
        let runtime = Runtime::new(RuntimeConfig {
            cache: CachePolicy::Lru(CacheConfig {
                max_bytes: 1024 * 1024,
                max_entries,
            }),
            ..RuntimeConfig::default()
        })?;
        let p = runtime.prepare(&graph).await?;
        let input = p.inputs().scalar(&cutoff, 1_i64.into())?.finish()?;
        assert_eq!(
            p.query(&[out], &[], &input)
                .await?
                .report()
                .source_executions,
            1
        );
        let hit = p.query(&[out], &[], &input).await?;
        assert_eq!(hit.report().source_executions, 0);
        assert_eq!(hit.report().physical_plans, 0);
        let next = input.edit().scalar(&cutoff, 2_i64.into())?.finish()?;
        let next = p.query(&[out], &[], &next).await?;
        assert_eq!(
            next.report().source_executions,
            usize::from(max_entries == 1)
        );
    }
    Ok(())
}

#[tokio::test]
async fn retention_bypass_disabled_and_signed_zero_keys() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let input = b.scalar_input("x", DataType::Float64)?;
    let node = b.add_scalar("x", input.expr_ref())?;
    let out = b.scalar_output("x", &node)?;
    let graph = b.finish()?;
    for policy in [
        CachePolicy::Disabled,
        CachePolicy::Lru(CacheConfig {
            max_bytes: 1,
            max_entries: 1,
        }),
        CachePolicy::default(),
    ] {
        let runtime = Runtime::new(RuntimeConfig {
            cache: policy,
            ..RuntimeConfig::default()
        })?;
        let p = runtime.prepare(&graph).await?;
        for x in [
            0.0_f64,
            -0.0,
            f64::from_bits(0x7ff8000000000001),
            f64::from_bits(0x7ff8000000000002),
        ] {
            let inputs = p
                .inputs()
                .scalar(&input, ScalarValue::Float64(Some(x)))?
                .finish()?;
            let result = p.query(&[], &[out], &inputs).await?;
            let ScalarValue::Float64(Some(value)) = result.scalar(&out)? else {
                panic!()
            };
            assert_eq!(value.to_bits(), x.to_bits());
        }
        assert!(runtime.cache_stats().bytes <= 128 * 1024 * 1024);
    }
    Ok(())
}

#[tokio::test]
async fn fixed_assets_survive_clearing_and_bindings_do_not_own_sources() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let asset = common::snapshot(&[1, 2]);
    let node = b.table_snapshot("asset", asset.clone())?;
    let out = b.table_output("asset", &node)?;
    let scalar = b.add_scalar("one", lit(1_i64))?;
    let scalar_out = b.scalar_output("one", &scalar)?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let p = runtime.prepare(&b.finish()?).await?;
    let inputs = p.inputs().finish()?;
    for _ in 0..2 {
        let result = p.query(&[out], &[scalar_out], &inputs).await?;
        assert_eq!(result.table(&out)?.id(), asset.id());
        p.clear_results();
    }
    assert_eq!(runtime.cache_stats().entries, 0);
    Ok(())
}

#[tokio::test]
async fn sources_cannot_hide_a_volatile_program_behind_a_view() -> Result<()> {
    use avenger_datafusion_dataflow::datafusion::{
        catalog::view::ViewTable, datasource::provider_as_source, functions::datetime::expr_fn::now,
    };
    use std::sync::Arc;
    let hidden = LogicalPlanBuilder::empty(true)
        .project(vec![now().alias("time")])?
        .build()?;
    let view = Arc::new(ViewTable::new(hidden, None));
    use avenger_datafusion_dataflow::datafusion::logical_expr::{LogicalPlan, TableScan};
    let source = provider_as_source(view);
    let scan = LogicalPlan::TableScan(TableScan::try_new(
        "view",
        source.clone(),
        None,
        vec![],
        None,
    )?);
    assert!(DataflowBuilder::new().add_plan("source", scan).is_err());
    // DataFusion's builder can inline the view; that visible program is safely analyzed.
    let expanded = LogicalPlanBuilder::scan("view", source, None)?.build()?;
    let mut b = DataflowBuilder::new();
    let node = b.add_plan("visible_program", expanded)?;
    b.table_output("out", &node)?;
    let p = Runtime::new(RuntimeConfig::default())?
        .prepare(&b.finish()?)
        .await?;
    assert_eq!(
        p.explain().nodes[0].reuse_scope,
        avenger_datafusion_dataflow::ReuseScope::EvaluationLocal
    );
    Ok(())
}
