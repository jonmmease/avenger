mod common;
use avenger_datafusion_dataflow::{
    arrow::datatypes::DataType,
    datafusion::logical_expr::{col, lit, LogicalPlanBuilder},
    CacheConfig, CachePolicy, DataflowBuilder, Error, ExecutionConfig, ExprInput, PreparedDataflow,
    PreparedExtension, Result, Runtime, RuntimeConfig, TableOutput,
};
use common::source::ControlledSource;
use std::sync::{atomic::Ordering, Arc};

async fn base_source(
    runtime: &Runtime,
    source: &Arc<ControlledSource>,
) -> Result<(PreparedDataflow, TableOutput)> {
    let mut b = DataflowBuilder::new();
    let node = b.add_plan("source", source.plan()?)?;
    let output = b.table_output("source", &node)?;
    Ok((runtime.prepare(&b.finish()?).await?, output))
}
async fn filtered(
    base: &PreparedDataflow,
    output: TableOutput,
) -> Result<(PreparedExtension, ExprInput, TableOutput)> {
    let mut a = DataflowBuilder::with_base(&base.interface());
    let imported = a.import_table("source", &output)?;
    let filter = a.expr_input("selected", DataType::Boolean)?;
    let node = a.add_plan(
        "filtered",
        LogicalPlanBuilder::from(imported.plan_ref())
            .filter(filter.expr_ref())?
            .build()?,
    )?;
    let out = a.table_output("out", &node)?;
    Ok((base.prepare_extension(&a.finish()?).await?, filter, out))
}

#[tokio::test]
async fn extensions_share_base_cache_and_clear_only_their_own_results() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let source = Arc::new(ControlledSource::new(&common::snapshot(&[1, 2, 3]))?);
    let (base, out) = base_source(&runtime, &source).await?;
    let inputs = base.inputs().finish()?;
    base.query(&[out], &[], &inputs).await?;
    assert_eq!(source.scans.load(Ordering::SeqCst), 1);
    let (a, filter_a, out_a) = filtered(&base, out).await?;
    let (b, filter_b, out_b) = filtered(&base, out).await?;
    let a_inputs = a
        .inputs()
        .expr(&filter_a, col("value").gt(lit(1_i64)))?
        .finish()?;
    let b_inputs = b
        .inputs()
        .expr(&filter_b, col("value").gt(lit(2_i64)))?
        .finish()?;
    assert_eq!(
        common::values(
            a.query(&[out_a], &[], &inputs, &a_inputs)
                .await?
                .table(&out_a)?
        ),
        [2, 3]
    );
    b.query(&[out_b], &[], &inputs, &b_inputs).await?;
    assert_eq!(source.scans.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.cache_stats().entries, 3);
    a.clear_results();
    assert_eq!(runtime.cache_stats().entries, 2);
    assert_eq!(
        b.query(&[out_b], &[], &inputs, &b_inputs)
            .await?
            .report()
            .physical_plans,
        0
    );
    assert_eq!(
        a.query(&[out_a], &[], &inputs, &a_inputs)
            .await?
            .report()
            .physical_plans,
        1
    );
    source.replace(&common::snapshot(&[4, 5]))?;
    base.clear_results();
    assert_eq!(runtime.cache_stats().entries, 0);
    assert_eq!(
        common::values(
            a.query(&[out_a], &[], &inputs, &a_inputs)
                .await?
                .table(&out_a)?
        ),
        [4, 5]
    );
    assert_eq!(
        common::values(
            b.query(&[out_b], &[], &inputs, &b_inputs)
                .await?
                .table(&out_b)?
        ),
        [4, 5]
    );
    assert_eq!(source.scans.load(Ordering::SeqCst), 2);
    drop(base);
    drop(a);
    assert_eq!(runtime.cache_stats().entries, 2); // Base and b are still retained.
    drop(b);
    assert_eq!(runtime.cache_stats().entries, 0);
    assert_eq!(runtime.cache_stats().bytes, 0);
    Ok(())
}

#[tokio::test]
async fn in_flight_clear_and_cancellation_release_limits_and_prevent_stale_publication(
) -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig {
        execution: ExecutionConfig {
            max_active_queries: 1,
            ..Default::default()
        },
        ..Default::default()
    })?;
    let source = Arc::new(ControlledSource::new(&common::snapshot(&[1, 2, 3]))?);
    source.gated.store(true, Ordering::SeqCst);
    let (base, out) = base_source(&runtime, &source).await?;
    let inputs = base.inputs().finish()?;
    let (extension, filter, output) = filtered(&base, out).await?;
    let local = extension.inputs().expr(&filter, lit(true))?.finish()?;
    let spawn = || {
        let (extension, inputs, local) = (extension.clone(), inputs.clone(), local.clone());
        tokio::spawn(async move { extension.query(&[output], &[], &inputs, &local).await })
    };
    let task = spawn();
    source.entered.notified().await;
    base.clear_results();
    source.release.notify_one();
    assert_eq!(task.await.unwrap()?.table(&output)?.num_rows(), 3);
    assert_eq!(runtime.cache_stats().entries, 0);
    let task = spawn();
    source.entered.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(runtime.cache_stats().entries, 0);
    source.gated.store(false, Ordering::SeqCst);
    extension.query(&[output], &[], &inputs, &local).await?;
    assert_eq!(runtime.cache_stats().entries, 2);
    assert_eq!(source.scans.load(Ordering::SeqCst), 3);
    Ok(())
}

#[tokio::test]
async fn transitive_bindings_invalidate_derived_results_and_keep_unrelated_values_out_of_keys(
) -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let mut b = DataflowBuilder::new();
    let table = b.table_input("rows", common::schema())?;
    let offset = b.scalar_input("offset", DataType::Int64)?;
    let filter = b.expr_input("filter", DataType::Boolean)?;
    let unrelated = b.scalar_input("unrelated", DataType::Int64)?;
    let source = b.add_plan(
        "source",
        LogicalPlanBuilder::from(table.plan_ref())
            .filter(filter.expr_ref())?
            .project(vec![(col("value") + offset.expr_ref()).alias("value")])?
            .build()?,
    )?;
    let output = b.table_output("source", &source)?;
    let base = runtime.prepare(&b.finish()?).await?;
    let (extension, local_filter, out) = filtered(&base, output).await?;
    let inputs = base
        .inputs()
        .table(&table, common::snapshot(&[1, 2]))?
        .scalar(&offset, 1_i64.into())?
        .expr(&filter, lit(true))?
        .scalar(&unrelated, 0_i64.into())?
        .finish()?;
    let local = extension
        .inputs()
        .expr(&local_filter, lit(true))?
        .finish()?;
    assert_eq!(
        common::values(
            extension
                .query(&[out], &[], &inputs, &local)
                .await?
                .table(&out)?
        ),
        [2, 3]
    );
    let unused = inputs.edit().scalar(&unrelated, 99_i64.into())?.finish()?;
    assert_eq!(
        extension
            .query(&[out], &[], &unused, &local)
            .await?
            .report()
            .physical_plans,
        0
    );
    let moved = unused.edit().scalar(&offset, 2_i64.into())?.finish()?;
    assert_eq!(
        common::values(
            extension
                .query(&[out], &[], &moved, &local)
                .await?
                .table(&out)?
        ),
        [3, 4]
    );
    let selected = moved
        .edit()
        .expr(&filter, col("value").gt(lit(1_i64)))?
        .finish()?;
    assert_eq!(
        common::values(
            extension
                .query(&[out], &[], &selected, &local)
                .await?
                .table(&out)?
        ),
        [4]
    );
    let replaced = selected
        .edit()
        .table(&table, common::snapshot(&[10]))?
        .finish()?;
    assert_eq!(
        common::values(
            extension
                .query(&[out], &[], &replaced, &local)
                .await?
                .table(&out)?
        ),
        [12]
    );
    assert_eq!(
        extension
            .query(&[out], &[], &inputs, &local)
            .await?
            .report()
            .physical_plans,
        0
    );
    Ok(())
}

#[tokio::test]
async fn eviction_disabled_cache_and_oversize_bypass_preserve_results() -> Result<()> {
    for policy in [
        CachePolicy::Disabled,
        CachePolicy::Lru(CacheConfig {
            max_bytes: 1,
            max_entries: 1,
        }),
        CachePolicy::Lru(CacheConfig {
            max_bytes: 1_000_000,
            max_entries: 1,
        }),
    ] {
        let runtime = Runtime::new(RuntimeConfig {
            cache: policy,
            ..Default::default()
        })?;
        let source = Arc::new(ControlledSource::new(&common::snapshot(&[1, 2, 3]))?);
        let (base, out) = base_source(&runtime, &source).await?;
        let inputs = base.inputs().finish()?;
        let (extension, filter, output) = filtered(&base, out).await?;
        for cutoff in [0_i64, 1, 0] {
            let local = extension
                .inputs()
                .expr(&filter, col("value").gt(lit(cutoff)))?
                .finish()?;
            assert_eq!(
                common::values(
                    extension
                        .query(&[output], &[], &inputs, &local)
                        .await?
                        .table(&output)?
                ),
                if cutoff == 0 {
                    vec![1, 2, 3]
                } else {
                    vec![2, 3]
                }
            );
        }
        assert_eq!(source.scans.load(Ordering::SeqCst), 3);
        assert!(runtime.cache_stats().entries <= 1);
    }
    Ok(())
}

#[tokio::test]
async fn invalid_reused_expression_fails_before_source_execution() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let source = Arc::new(ControlledSource::new(&common::snapshot(&[1]))?);
    let mut b = DataflowBuilder::new();
    let predicate = b.expr_input("predicate", DataType::Boolean)?;
    let rows = b.add_plan("source", source.plan()?)?;
    let out = b.table_output("source", &rows)?;
    let base = runtime.prepare(&b.finish()?).await?;
    let mut a = DataflowBuilder::with_base(&base.interface());
    let rows = a.import_table("source", &out)?;
    let node = a.add_plan(
        "selected",
        LogicalPlanBuilder::from(rows.plan_ref())
            .filter(predicate.expr_ref())?
            .build()?,
    )?;
    let out = a.table_output("out", &node)?;
    let extension = base.prepare_extension(&a.finish()?).await?;
    let inputs = base
        .inputs()
        .expr(&predicate, col("missing").gt(lit(0_i64)))?
        .finish()?;
    assert!(matches!(
        extension
            .query(&[out], &[], &inputs, &extension.inputs().finish()?)
            .await,
        Err(Error::InvalidExprInput { .. })
    ));
    assert_eq!(source.scans.load(Ordering::SeqCst), 0);
    assert_eq!(runtime.cache_stats().entries, 0);
    Ok(())
}

#[tokio::test]
async fn extension_hits_charge_the_shared_active_budget() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let input = b.table_input("input", common::schema())?;
    let source = b.add_plan("source", input.plan_ref())?;
    let out = b.table_output("out", &source)?;
    let runtime = Runtime::new(RuntimeConfig {
        execution: ExecutionConfig {
            max_active_queries: 1,
            max_materialized_bytes: 12_000,
        },
        ..Default::default()
    })?;
    let base = runtime.prepare(&b.finish()?).await?;
    let inputs = base
        .inputs()
        .table(&input, common::snapshot(&(0..1000).collect::<Vec<_>>()))?
        .finish()?;
    let mut a = DataflowBuilder::with_base(&base.interface());
    // Direct input readers exercise the same shared budget without allocating a
    // second full table upstream while warming one result.
    let one = a.add_plan("one", input.plan_ref())?;
    let two = a.add_plan("two", input.plan_ref())?;
    let one = a.table_output("one", &one)?;
    let two = a.table_output("two", &two)?;
    let extension = base.prepare_extension(&a.finish()?).await?;
    let local = extension.inputs().finish()?;
    extension.query(&[one], &[], &inputs, &local).await?;
    extension.query(&[two], &[], &inputs, &local).await?;
    assert!(matches!(
        extension.query(&[one, two], &[], &inputs, &local).await,
        Err(Error::ResourceExhausted { .. })
    ));
    assert_eq!(
        extension
            .query(&[one], &[], &inputs, &local)
            .await?
            .report()
            .cache_hits,
        1
    );
    base.query(&[out], &[], &inputs).await?;
    Ok(())
}

#[tokio::test]
async fn preaggregation_serves_new_active_values_and_tracks_only_fixed_dependencies() -> Result<()>
{
    use avenger_datafusion_dataflow::{
        arrow::array::Int64Array,
        datafusion::functions_aggregate::expr_fn::{count, sum},
    };
    use std::collections::BTreeMap;
    let values = [1_i64, 1, 2, 2, 3, 4, 5];
    let source = Arc::new(ControlledSource::new(&common::snapshot(&values))?);
    let runtime = Runtime::new(Default::default())?;
    let mut b = DataflowBuilder::new();
    let source_node = b.add_plan("source", source.plan()?)?;
    let source_out = b.table_output("source", &source_node)?;
    let fixed = b.scalar_input("fixed_limit", DataType::Int64)?;
    let base = runtime.prepare(&b.finish()?).await?;
    let mut a = DataflowBuilder::with_base(&base.interface());
    let source_node = a.import_table("source", &source_out)?;
    let active = a.expr_input("active", DataType::Boolean)?;
    let preaggregate = a.add_plan(
        "preaggregate",
        LogicalPlanBuilder::from(source_node.plan_ref())
            .filter(col("value").lt_eq(fixed.expr_ref()))?
            .project(vec![
                col("value"),
                (col("value") % lit(2_i64)).alias("bucket"),
            ])?
            .aggregate(
                vec![col("bucket"), col("value")],
                vec![count(lit(1_i64)).alias("n")],
            )?
            .build()?,
    )?;
    let counts = a.add_plan(
        "counts",
        LogicalPlanBuilder::from(preaggregate.plan_ref())
            .filter(active.expr_ref())?
            .aggregate(vec![col("bucket")], vec![sum(col("n")).alias("count")])?
            .build()?,
    )?;
    let output = a.table_output("counts", &counts)?;
    let extension = base.prepare_extension(&a.finish()?).await?;
    for (fixed_value, threshold, physical_plans) in [
        (5_i64, 0_i64, 3),
        (5, 2, 1),
        (5, 0, 0),
        (3, 0, 2),
        (5, 0, 0),
        (5, 20, 1),
    ] {
        let base_inputs = base.inputs().scalar(&fixed, fixed_value.into())?.finish()?;
        let local = extension
            .inputs()
            .expr(&active, col("value").gt(lit(threshold)))?
            .finish()?;
        let result = extension
            .query(&[output], &[], &base_inputs, &local)
            .await?;
        let mut actual = BTreeMap::new();
        for batch in result.table(&output)?.batches() {
            let bucket = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            let count = batch
                .column(1)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            for row in 0..batch.num_rows() {
                actual.insert(bucket.value(row), count.value(row));
            }
        }
        let mut expected = BTreeMap::new();
        for value in values {
            if value <= fixed_value && value > threshold {
                *expected.entry(value % 2).or_insert(0_i64) += 1;
            }
        }
        assert_eq!(actual, expected);
        assert_eq!(result.report().physical_plans, physical_plans);
        assert_eq!(source.scans.load(Ordering::SeqCst), 1);
    }
    Ok(())
}
