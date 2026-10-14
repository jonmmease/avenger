mod common;
use avenger_datafusion_dataflow::{
    datafusion::{
        datasource::{provider_as_source, MemTable},
        logical_expr::LogicalPlanBuilder,
    },
    CachePolicy, DataflowBuilder, Error, ExecutionConfig, Result, Runtime, RuntimeConfig,
};
use common::source::ControlledSource;
use std::sync::{atomic::Ordering, Arc};

#[tokio::test]
async fn clearing_and_cancelling_in_flight_misses_cannot_repopulate_results() -> Result<()> {
    let data = common::snapshot(&[1, 2, 3]);
    let source = Arc::new(ControlledSource::new(&data)?);
    source.gated.store(true, Ordering::SeqCst);
    let mut b = DataflowBuilder::new();
    let node = b.add_plan(
        "source",
        LogicalPlanBuilder::scan("source", provider_as_source(source.clone()), None)?.build()?,
    )?;
    let out = b.table_output("out", &node)?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let prepared = runtime.prepare(&b.finish()?).await?;
    assert_eq!(source.scans.load(Ordering::SeqCst), 0);
    let p = prepared.clone();
    let task = tokio::spawn(async move { p.query(&[out], &[], &p.inputs().finish()?).await });
    source.entered.notified().await;
    prepared.clear_results();
    source.release.notify_one();
    assert_eq!(task.await.unwrap()?.table(&out)?.num_rows(), 3);
    assert_eq!(runtime.cache_stats().entries, 0);
    let p = prepared.clone();
    let task = tokio::spawn(async move { p.query(&[out], &[], &p.inputs().finish()?).await });
    source.entered.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(runtime.cache_stats().entries, 0);
    source.gated.store(false, Ordering::SeqCst);
    prepared
        .query(&[out], &[], &prepared.inputs().finish()?)
        .await?;
    assert_eq!(runtime.cache_stats().entries, 1);
    assert_eq!(source.scans.load(Ordering::SeqCst), 3);
    drop(prepared);
    assert_eq!(runtime.cache_stats().entries, 0);
    Ok(())
}

#[tokio::test]
async fn cache_hits_charge_active_budget_and_failure_preserves_retention() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let input = b.table_input("input", common::schema())?;
    let a = b.add_plan("a", input.plan_ref())?;
    let bnode = b.add_plan("b", input.plan_ref())?;
    let a = b.table_output("a", &a)?;
    let bout = b.table_output("b", &bnode)?;
    let runtime = Runtime::new(RuntimeConfig {
        execution: ExecutionConfig {
            max_active_queries: 1,
            max_materialized_bytes: 12_000,
        },
        ..RuntimeConfig::default()
    })?;
    let p = runtime.prepare(&b.finish()?).await?;
    let values = (0..1000).collect::<Vec<_>>();
    let inputs = p
        .inputs()
        .table(&input, common::snapshot(&values))?
        .finish()?;
    p.query(&[a], &[], &inputs).await?;
    p.query(&[bout], &[], &inputs).await?;
    assert_eq!(runtime.cache_stats().entries, 2);
    assert!(matches!(
        p.query(&[a, bout], &[], &inputs).await,
        Err(Error::ResourceExhausted { .. })
    ));
    assert_eq!(runtime.cache_stats().entries, 2);
    assert_eq!(p.query(&[a], &[], &inputs).await?.report().cache_hits, 1);
    Ok(())
}

#[tokio::test]
async fn bindings_and_interface_do_not_retain_graph_owned_source_buffers() -> Result<()> {
    let data = common::snapshot(&[1, 2, 3]);
    let source = Arc::new(MemTable::try_new(
        data.schema().clone(),
        vec![data.batches().to_vec()],
    )?);
    let weak = Arc::downgrade(&source);
    let mut b = DataflowBuilder::new();
    let node = b.add_plan(
        "source",
        LogicalPlanBuilder::scan("source", provider_as_source(source), None)?.build()?,
    )?;
    b.table_output("out", &node)?;
    let definition = b.finish()?;
    let interface = definition.interface();
    let mut additional = DataflowBuilder::with_base(&interface);
    additional.import_table("source", &interface.root().table_output("out")?)?;
    let additional = additional.finish()?;
    let runtime = Runtime::new(RuntimeConfig {
        cache: CachePolicy::Disabled,
        ..RuntimeConfig::default()
    })?;
    let p = runtime.prepare(&definition).await?;
    let inputs = p.inputs().finish()?;
    drop(p);
    drop(definition);
    assert!(weak.upgrade().is_none());
    assert_eq!(additional.num_nodes(), 0);
    assert!(interface.root().table_output("out").is_ok());
    let _ = inputs.edit();
    Ok(())
}

#[tokio::test]
async fn retained_expression_inputs_do_not_retain_prepared_cache_namespaces() -> Result<()> {
    use avenger_datafusion_dataflow::{
        arrow::datatypes::DataType,
        datafusion::logical_expr::{col, lit},
    };
    let mut b = DataflowBuilder::new();
    let data = b.table_snapshot("data", common::snapshot(&[1, 2, 3]))?;
    let expr = b.expr_input("selection", DataType::Boolean)?;
    let node = b.add_plan(
        "selected",
        LogicalPlanBuilder::from(data.plan_ref())
            .filter(expr.expr_ref())?
            .build()?,
    )?;
    let out = b.table_output("out", &node)?;
    let runtime = Runtime::new(Default::default())?;
    let p = runtime.prepare(&b.finish()?).await?;
    let inputs = p
        .inputs()
        .expr(&expr, col("value").gt(lit(1_i64)))?
        .finish()?;
    p.query(&[out], &[], &inputs).await?;
    assert_eq!(runtime.cache_stats().entries, 1);
    drop(p);
    assert_eq!(runtime.cache_stats().entries, 0);
    assert!(inputs.edit().expr(&expr, lit(true))?.finish().is_ok());
    Ok(())
}
