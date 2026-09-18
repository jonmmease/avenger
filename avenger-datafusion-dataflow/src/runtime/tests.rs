use super::*;
use crate::{CacheConfig, CachePolicy};
use datafusion::functions_aggregate::expr_fn::sum;
use datafusion::logical_expr::LogicalPlanBuilder;
use datafusion::logical_expr::{col, Expr};
use std::time::Duration;

#[path = "../../tests/common/mod.rs"]
pub(super) mod common;
use common::source::ControlledSource;

pub(super) async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("calculation reached the required state");
}

pub(super) async fn watching(p: &PreparedDataflow, out: TableOutput, count: usize) {
    let index = p.inner.graph.outputs[out.index].node;
    until(|| {
        p.inner
            .runtime
            .cache
            .lock()
            .unwrap()
            .watchers(p.inner.namespace, index)
            == count
    })
    .await;
}

pub(super) async fn idle(runtime: &Runtime) {
    until(|| {
        runtime.inner.cache.lock().unwrap().flights.is_empty()
            && runtime.inner.active_bytes.load(Ordering::Relaxed) == 0
    })
    .await;
}

async fn entered(source: &ControlledSource) {
    tokio::time::timeout(Duration::from_secs(10), source.entered.notified())
        .await
        .expect("source enters planning");
}

async fn source_graph(
    runtime: &Runtime,
) -> Result<(Arc<ControlledSource>, PreparedDataflow, TableOutput)> {
    let source = Arc::new(ControlledSource::new(&common::snapshot(&[1, 2, 3]))?);
    source.gated.store(true, Ordering::SeqCst);
    let mut b = crate::DataflowBuilder::new();
    let node = b.add_plan("source", source.plan()?)?;
    let out = b.table_output("source", &node)?;
    let p = runtime.prepare(&b.finish()?).await?;
    Ok((source, p, out))
}

pub(super) fn query(
    p: &PreparedDataflow,
    out: TableOutput,
) -> tokio::task::JoinHandle<Result<DataflowResult>> {
    let p = p.clone();
    tokio::spawn(async move { p.query(&[out], &[], &p.inputs().finish()?).await })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overlapping_queries_share_without_retention_and_survive_initiator_cancellation(
) -> Result<()> {
    for policy in [
        CachePolicy::default(),
        CachePolicy::Disabled,
        CachePolicy::Lru(CacheConfig {
            max_bytes: 1,
            max_entries: 1,
        }),
    ] {
        let runtime = Runtime::new(RuntimeConfig {
            cache: policy,
            ..Default::default()
        })?;
        let (source, p, out) = source_graph(&runtime).await?;
        let first = query(&p, out);
        entered(&source).await;
        let second = query(&p, out);
        watching(&p, out, 2).await;
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        watching(&p, out, 1).await;
        source.release.notify_one();
        let result = second.await.unwrap()?;
        assert_eq!(common::values(result.table(&out)?), [1, 2, 3]);
        assert_eq!(result.report().in_flight_hits, 1);
        assert_eq!(result.report().physical_plans, 0);
        assert_eq!(source.scans.load(Ordering::SeqCst), 1);
        idle(&runtime).await;
        source.gated.store(false, Ordering::SeqCst);
        let retained = runtime.cache_stats().entries > 0;
        query(&p, out).await.unwrap()?;
        assert_eq!(
            source.scans.load(Ordering::SeqCst),
            if retained { 1 } else { 2 }
        );
    }
    Ok(())
}

#[tokio::test]
async fn cancelling_all_consumers_releases_transitive_work_but_preserves_shared_parent(
) -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let shared = Arc::new(ControlledSource::new(&common::snapshot(&[1, 2, 3]))?);
    let private = Arc::new(ControlledSource::new(&common::snapshot(&[4]))?);
    shared.gated.store(true, Ordering::SeqCst);
    private.gated.store(true, Ordering::SeqCst);
    let mut b = crate::DataflowBuilder::new();
    let shared_node = b.add_plan("shared", shared.plan()?)?;
    let shared_out = b.table_output("shared", &shared_node)?;
    let parent = b.add_plan(
        "parent",
        LogicalPlanBuilder::from(private.plan()?)
            .union(shared_node.plan_ref())?
            .build()?,
    )?;
    let child = b.add_plan("child", parent.plan_ref())?;
    let child_out = b.table_output("child", &child)?;
    let p = runtime.prepare(&b.finish()?).await?;
    let survivor = query(&p, shared_out);
    shared.entered.notified().await;
    let cancel = query(&p, child_out);
    // The parent waits on the shared dependency before entering its private scan.
    watching(&p, shared_out, 2).await;
    cancel.abort();
    assert!(cancel.await.unwrap_err().is_cancelled());
    watching(&p, shared_out, 1).await;
    assert_eq!(private.scans.load(Ordering::SeqCst), 0);
    shared.release.notify_one();
    survivor.await.unwrap()?;
    idle(&runtime).await;
    assert_eq!(shared.scans.load(Ordering::SeqCst), 1);

    p.clear_results();
    let cancel = query(&p, child_out);
    shared.entered.notified().await;
    shared.release.notify_one();
    private.entered.notified().await;
    cancel.abort();
    assert!(cancel.await.unwrap_err().is_cancelled());
    idle(&runtime).await;
    private.gated.store(false, Ordering::SeqCst);
    let result = query(&p, child_out).await.unwrap()?;
    assert_eq!(result.table(&child_out)?.num_rows(), 4);
    assert_eq!(private.scans.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn warmup_and_final_queries_share_materialization_across_bindings_and_outputs() -> Result<()>
{
    for policy in [CachePolicy::default(), CachePolicy::Disabled] {
        let runtime = Runtime::new(RuntimeConfig {
            cache: policy,
            ..Default::default()
        })?;
        let source = Arc::new(ControlledSource::new(&common::snapshot(&[1, 2, 3]))?);
        source.gated.store(true, Ordering::SeqCst);
        let mut b = crate::DataflowBuilder::new();
        let cutoff = b.scalar_input("cutoff", datafusion::arrow::datatypes::DataType::Int64)?;
        let materialized = b.add_plan("materialized", source.plan()?)?;
        let warm = b.table_output("warm", &materialized)?;
        let final_node = b.add_plan(
            "final",
            LogicalPlanBuilder::from(materialized.plan_ref())
                .filter(col("value").gt(cutoff.expr_ref()))?
                .aggregate(Vec::<Expr>::new(), vec![sum(col("value")).alias("total")])?
                .build()?,
        )?;
        let out = b.table_output("out", &final_node)?;
        let p = runtime.prepare(&b.finish()?).await?;
        let before = p.inputs().scalar(&cutoff, 0_i64.into())?.finish()?;
        let after = before.edit().scalar(&cutoff, 1_i64.into())?.finish()?;
        let warm_task = {
            let p = p.clone();
            tokio::spawn(async move { p.query(&[warm], &[], &before).await })
        };
        entered(&source).await;
        let final_task = {
            let p = p.clone();
            tokio::spawn(async move { p.query(&[out, warm], &[], &after).await })
        };
        watching(&p, warm, 2).await;
        source.release.notify_one();
        let warm_result = warm_task.await.unwrap()?;
        let result = final_task.await.unwrap()?;
        assert_eq!(result.table(&out)?.num_rows(), 1);
        assert_eq!(
            ScalarValue::try_from_array(result.table(&out)?.batches()[0].column(0), 0)?,
            5_i64.into()
        );
        assert_eq!(source.scans.load(Ordering::SeqCst), 1);
        assert_eq!(
            warm_result.report().physical_plans + result.report().physical_plans,
            2
        );
        assert_eq!(result.report().in_flight_hits, 1);
        idle(&runtime).await;
    }
    Ok(())
}

fn reserve(
    p: &PreparedDataflow,
    out: TableOutput,
) -> (crate::in_flight::Reservation, crate::in_flight::Pending) {
    let mut cache = p.inner.runtime.cache.lock().unwrap();
    let epoch = cache.epoch(p.inner.namespace);
    let key = crate::cache::ValueKey {
        namespace: p.inner.namespace,
        node: p.inner.graph.outputs[out.index].node,
        instance: None,
        inputs: vec![],
        base_inputs: vec![],
        upstream: None,
    };
    let crate::in_flight::Lookup::Reserved(reservation, pending) =
        cache.lookup(key, epoch, &p.inner.runtime, "source".into())
    else {
        panic!("fresh reservation")
    };
    (reservation, pending)
}

#[tokio::test]
async fn renewed_interest_waits_for_settlement_then_converges_on_one_replacement() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let (source, p, out) = source_graph(&runtime).await?;
    let (reservation, pending) = reserve(&p, out);
    drop(pending);
    reservation.unwatched().await;
    let a = query(&p, out);
    let b = query(&p, out);
    watching(&p, out, 2).await;
    assert_eq!(source.scans.load(Ordering::SeqCst), 0);
    reservation.cancel();
    entered(&source).await;
    watching(&p, out, 2).await;
    assert_eq!(source.scans.load(Ordering::SeqCst), 1);
    source.release.notify_one();
    a.await.unwrap()?;
    b.await.unwrap()?;
    idle(&runtime).await;
    Ok(())
}

#[tokio::test]
async fn renewed_interest_can_prevent_cancellation_and_failures_are_not_retried() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let (source, p, out) = source_graph(&runtime).await?;
    let (reservation, pending) = reserve(&p, out);
    drop(pending);
    let a = query(&p, out);
    let b = query(&p, out);
    watching(&p, out, 2).await;
    assert!(futures::poll!(Box::pin(reservation.unwatched())).is_pending());
    reservation.finish(Err(Error::ResourceExhausted { limit: 42 }), |_| {});
    for task in [a, b] {
        let error = task.await.unwrap().unwrap_err();
        match error {
            Error::Shared(error) => {
                assert!(matches!(*error, Error::ResourceExhausted { limit: 42 }))
            }
            Error::ResourceExhausted { limit: 42 } => {}
            e => panic!("unexpected {e}"),
        }
    }
    assert_eq!(source.scans.load(Ordering::SeqCst), 0);
    let (reservation, pending) = reserve(&p, out);
    let a = query(&p, out);
    watching(&p, out, 2).await;
    drop(pending);
    drop(reservation);
    assert!(a
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("without a result"));
    assert_eq!(source.scans.load(Ordering::SeqCst), 0);
    source.gated.store(false, Ordering::SeqCst);
    query(&p, out).await.unwrap()?;
    idle(&runtime).await;
    Ok(())
}

#[tokio::test]
async fn success_after_cancellation_is_delivered_with_normal_epoch_and_retention_rules(
) -> Result<()> {
    for (policy, clear, retained) in [
        (CachePolicy::default(), false, 1),
        (CachePolicy::Disabled, false, 0),
        (CachePolicy::default(), true, 0),
    ] {
        let runtime = Runtime::new(RuntimeConfig {
            cache: policy,
            ..Default::default()
        })?;
        let (source, p, out) = source_graph(&runtime).await?;
        let (reservation, pending) = reserve(&p, out);
        drop(pending);
        reservation.unwatched().await;
        let a = query(&p, out);
        watching(&p, out, 1).await;
        if clear {
            p.clear_results();
        }
        let budget = Arc::new(Reservation {
            runtime: runtime.inner.clone(),
            bytes: AtomicUsize::new(0),
        });
        let value = MaterializedValue::Table(common::snapshot(&[9]));
        budget.charge(value.size())?;
        let value = Arc::new(NodeValue {
            namespace: p.inner.namespace,
            index: p.inner.graph.outputs[out.index].node,
            instance: None,
            value,
            dependencies: vec![],
            budget,
        });
        reservation.finish(Ok(value), |_| {});
        let result = a.await.unwrap()?;
        assert_eq!(common::values(result.table(&out)?), [9]);
        assert_eq!(source.scans.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.cache_stats().entries, retained);
        idle(&runtime).await;
    }
    Ok(())
}

#[tokio::test]
async fn base_queries_and_independent_extensions_share_only_the_base_producer() -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig {
        cache: CachePolicy::Disabled,
        ..Default::default()
    })?;
    let (source, base, out) = source_graph(&runtime).await?;
    let first = query(&base, out);
    entered(&source).await;
    let mut followers = vec![];
    for minimum in [1_i64, 2_i64] {
        let mut b = crate::DataflowBuilder::with_base(&base.interface());
        let imported = b.import_table("base", &out)?;
        let node = b.add_plan(
            "filtered",
            LogicalPlanBuilder::from(imported.plan_ref())
                .filter(col("value").gt(datafusion::logical_expr::lit(minimum)))?
                .build()?,
        )?;
        let output = b.table_output("out", &node)?;
        let extension = base.prepare_extension(&b.finish()?).await?;
        let inputs = base.inputs().finish()?;
        let local = extension.inputs().finish()?;
        followers.push(tokio::spawn(async move {
            let result = extension.query(&[output], &[], &inputs, &local).await?;
            Ok::<_, Error>((
                common::values(result.table(&output)?),
                result.report().in_flight_hits,
            ))
        }));
    }
    watching(&base, out, 3).await;
    first.abort();
    let _ = first.await;
    source.release.notify_one();
    for (task, expected) in followers.into_iter().zip([vec![2, 3], vec![3]]) {
        let (values, joins) = task.await.unwrap()?;
        assert_eq!(values, expected);
        assert_eq!(joins, 1);
    }
    assert_eq!(source.scans.load(Ordering::SeqCst), 1);
    idle(&runtime).await;
    Ok(())
}

#[tokio::test]
async fn changed_bindings_preparations_and_clear_epochs_do_not_join() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let source = Arc::new(ControlledSource::new(&common::snapshot(&[1, 2, 3]))?);
    source.gated.store(true, Ordering::SeqCst);
    let mut b = crate::DataflowBuilder::new();
    let offset = b.scalar_input("offset", datafusion::arrow::datatypes::DataType::Int64)?;
    let node = b.add_plan(
        "source",
        LogicalPlanBuilder::from(source.plan()?)
            .project(vec![(col("value") + offset.expr_ref()).alias("value")])?
            .build()?,
    )?;
    let out = b.table_output("source", &node)?;
    let flow = b.finish()?;
    let p = runtime.prepare(&flow).await?;
    let independent = runtime.prepare(&flow).await?;
    let spawn = |p: PreparedDataflow, value: i64| {
        let offset = offset.clone();
        tokio::spawn(async move {
            let inputs = p.inputs().scalar(&offset, value.into())?.finish()?;
            p.query(&[out], &[], &inputs).await
        })
    };
    let first = spawn(p.clone(), 0);
    entered(&source).await;
    let changed = spawn(p.clone(), 10);
    entered(&source).await;
    let different = spawn(independent, 0);
    entered(&source).await;
    p.clear_results();
    let current = spawn(p.clone(), 0);
    entered(&source).await;
    assert_eq!(source.scans.load(Ordering::SeqCst), 4);
    source.release.notify_waiters();
    for (task, expected) in [
        (first, vec![1, 2, 3]),
        (changed, vec![11, 12, 13]),
        (different, vec![1, 2, 3]),
        (current, vec![1, 2, 3]),
    ] {
        let result = task.await.unwrap()?;
        assert_eq!(common::values(result.table(&out)?), expected);
        assert_eq!(result.report().in_flight_hits, 0);
    }
    idle(&runtime).await;
    assert_eq!(
        query_with_scalar(&p, out, &offset, 0)
            .await?
            .report()
            .cache_hits,
        1
    );
    Ok(())
}

async fn query_with_scalar(
    p: &PreparedDataflow,
    out: TableOutput,
    input: &crate::ScalarInput,
    value: i64,
) -> Result<DataflowResult> {
    p.query(
        &[out],
        &[],
        &p.inputs().scalar(input, value.into())?.finish()?,
    )
    .await
}

#[tokio::test]
async fn scoped_nodes_share_matching_instances_and_keep_local_overrides_separate() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let source = Arc::new(ControlledSource::new(&common::snapshot(&[10]))?);
    source.gated.store(true, Ordering::SeqCst);
    let mut b = crate::DataflowBuilder::new();
    let rows = b.table_snapshot("rows", common::snapshot(&[1, 2]))?;
    let (scope, (offset, out)) =
        b.partition_by("panels", rows.plan_ref(), vec![col("value")], |s| {
            let offset = s.scalar_input("offset", datafusion::arrow::datatypes::DataType::Int64)?;
            let node = s.add_plan(
                "rows",
                LogicalPlanBuilder::from(s.rows().plan_ref())
                    .union(source.plan()?)?
                    .project(vec![(col("value") + offset.expr_ref()).alias("value")])?
                    .build()?,
            )?;
            Ok((offset, s.table_output("rows", &node)?))
        })?;
    let p = runtime.prepare(&b.finish()?).await?;
    let inputs = p
        .inputs()
        .scope_defaults(&scope, |b| b.scalar(&offset, 0_i64.into()))?
        .finish()?;
    let changed = inputs
        .edit()
        .at(&scope.instance([2_i64.into()])?, |b| {
            b.scalar(&offset, 100_i64.into())
        })?
        .finish()?;
    let a = {
        let p = p.clone();
        tokio::spawn(async move { p.query(&[out], &[], &inputs).await })
    };
    entered(&source).await;
    let b = {
        let p = p.clone();
        tokio::spawn(async move { p.query(&[out], &[], &changed).await })
    };
    watching(&p, out, 2).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            // Instance order is unspecified. Release whichever facets are waiting.
            source.release.notify_waiters();
            if a.is_finished() && b.is_finished() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both facet queries finish");
    assert_eq!(source.scans.load(Ordering::SeqCst), 3);
    let a = a.await.unwrap()?;
    let b = b.await.unwrap()?;
    for (result, second) in [(&a, vec![2, 10]), (&b, vec![102, 110])] {
        let first = scope.key([1_i64.into()])?;
        let last = scope.key([2_i64.into()])?;
        let mut values = common::values(result.scope(&scope)?.get(&first).unwrap().table(&out)?);
        values.sort();
        assert_eq!(values, [1, 10]);
        let mut values = common::values(result.scope(&scope)?.get(&last).unwrap().table(&out)?);
        values.sort();
        assert_eq!(values, second);
    }

    idle(&runtime).await;
    Ok(())
}

#[tokio::test]
async fn evaluation_local_scalars_are_fresh_while_reusable_ancestors_share() -> Result<()> {
    use datafusion::arrow::datatypes::DataType;
    use datafusion::logical_expr::{create_udf, scalar_subquery, ColumnarValue, Volatility};
    for volatility in [Volatility::Stable, Volatility::Volatile] {
        let runtime = Runtime::new(Default::default())?;
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let udf = create_udf(
            "counter",
            vec![DataType::Int64],
            DataType::Int64,
            volatility,
            Arc::new(move |_| {
                Ok(ColumnarValue::Scalar(
                    (counter.fetch_add(1, Ordering::SeqCst) as i64).into(),
                ))
            }),
        );
        let source = Arc::new(ControlledSource::new(&common::snapshot(&[1]))?);
        source.gated.store(true, Ordering::SeqCst);
        let mut b = crate::DataflowBuilder::new();
        let node = b.add_plan("source", source.plan()?)?;
        let source_out = b.table_output("source", &node)?;
        let scalar = b.add_scalar(
            "draw",
            udf.call(vec![scalar_subquery(Arc::new(node.plan_ref()))]),
        )?;
        let out = b.scalar_output("draw", &scalar)?;
        let descendant = b.add_scalar("copy", scalar.expr_ref())?;
        let copied = b.scalar_output("copy", &descendant)?;
        let p = runtime.prepare(&b.finish()?).await?;
        let spawn = || {
            let p = p.clone();
            tokio::spawn(async move { p.query(&[], &[out, copied], &p.inputs().finish()?).await })
        };
        let a = spawn();
        entered(&source).await;
        let b = spawn();
        watching(&p, source_out, 2).await;
        source.release.notify_one();
        let a = a.await.unwrap()?;
        let b = b.await.unwrap()?;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_ne!(a.scalar(&out)?, b.scalar(&out)?);
        assert_eq!(a.scalar(&out)?, a.scalar(&copied)?);
        assert_eq!(b.scalar(&out)?, b.scalar(&copied)?);
        assert_eq!(source.scans.load(Ordering::SeqCst), 1);
        idle(&runtime).await;
    }
    Ok(())
}

#[tokio::test]
async fn dropping_owned_futures_and_waiting_requests_cancels_but_dropping_task_handles_detaches(
) -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig {
        execution: ExecutionConfig {
            max_active_queries: 1,
            ..Default::default()
        },
        ..Default::default()
    })?;
    let (source, p, out) = source_graph(&runtime).await?;
    let inputs = p.inputs().finish()?;
    let outputs = [out];
    let mut future = Box::pin(p.query(&outputs, &[], &inputs));
    tokio::select! {
        _ = source.entered.notified() => {},
        result = &mut future => panic!("source must wait: {result:?}"),
    }
    let mut waiting = Box::pin(p.query(&outputs, &[], &inputs));
    assert!(futures::poll!(&mut waiting).is_pending());
    drop(waiting);
    assert_eq!(source.scans.load(Ordering::SeqCst), 1);
    drop(future);
    idle(&runtime).await;
    let detached = query(&p, out);
    entered(&source).await;
    drop(detached);
    watching(&p, out, 1).await;
    source.release.notify_one();
    idle(&runtime).await;
    assert_eq!(runtime.cache_stats().entries, 1);
    assert_eq!(query(&p, out).await.unwrap()?.report().cache_hits, 1);
    Ok(())
}

#[tokio::test]
async fn shared_results_charge_each_consumer_and_release_failed_consumer_reservations() -> Result<()>
{
    let runtime = Runtime::new(RuntimeConfig {
        execution: ExecutionConfig {
            max_materialized_bytes: 16_000,
            ..Default::default()
        },
        ..Default::default()
    })?;
    let (source, p, out) = source_graph(&runtime).await?;
    source.replace(&common::snapshot(&(0..1000).collect::<Vec<_>>()))?;
    let first = query(&p, out);
    entered(&source).await;
    let second = query(&p, out);
    watching(&p, out, 2).await;
    source.release.notify_one();
    let result = first.await.unwrap()?;
    assert_eq!(result.table(&out)?.num_rows(), 1000);
    assert!(matches!(
        second.await.unwrap(),
        Err(Error::ResourceExhausted { .. })
    ));
    idle(&runtime).await;
    source.gated.store(false, Ordering::SeqCst);
    assert_eq!(query(&p, out).await.unwrap()?.report().cache_hits, 1);
    idle(&runtime).await;
    Ok(())
}

#[tokio::test]
async fn reusable_scalar_calculations_share_without_completed_retention() -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig {
        cache: CachePolicy::Disabled,
        ..Default::default()
    })?;
    let source = Arc::new(ControlledSource::new(&common::snapshot(&[1, 2, 3]))?);
    source.gated.store(true, Ordering::SeqCst);
    let mut b = crate::DataflowBuilder::new();
    let plan = LogicalPlanBuilder::from(source.plan()?)
        .aggregate(Vec::<Expr>::new(), vec![sum(col("value"))])?
        .build()?;
    let node = b.add_scalar(
        "total",
        datafusion::logical_expr::scalar_subquery(Arc::new(plan)),
    )?;
    let out = b.scalar_output("total", &node)?;
    let p = runtime.prepare(&b.finish()?).await?;
    let spawn = || {
        let p = p.clone();
        tokio::spawn(async move { p.query(&[], &[out], &p.inputs().finish()?).await })
    };
    let first = spawn();
    entered(&source).await;
    let second = spawn();
    until(|| {
        runtime
            .inner
            .cache
            .lock()
            .unwrap()
            .watchers(p.inner.namespace, p.inner.graph.outputs[out.index].node)
            == 2
    })
    .await;
    source.release.notify_one();
    let first = first.await.unwrap()?;
    let second = second.await.unwrap()?;
    assert_eq!(first.scalar(&out)?, &6_i64.into());
    assert_eq!(second.scalar(&out)?, &6_i64.into());
    assert_eq!(first.report().physical_plans, 1);
    assert_eq!(second.report().in_flight_hits, 1);
    assert_eq!(source.scans.load(Ordering::SeqCst), 1);
    idle(&runtime).await;
    Ok(())
}

#[tokio::test]
async fn a_cancelled_renewed_consumer_does_not_restart_the_abandoned_calculation() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let (source, p, out) = source_graph(&runtime).await?;
    let (reservation, pending) = reserve(&p, out);
    drop(pending);
    reservation.unwatched().await;
    let consumer = query(&p, out);
    watching(&p, out, 1).await;
    consumer.abort();
    assert!(consumer.await.unwrap_err().is_cancelled());
    watching(&p, out, 0).await;
    reservation.cancel();
    idle(&runtime).await;
    assert_eq!(source.scans.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn producer_panics_settle_every_waiter_and_a_later_request_can_retry() -> Result<()> {
    use datafusion::arrow::datatypes::DataType;
    use datafusion::logical_expr::{create_udf, Volatility};
    let runtime = Runtime::new(Default::default())?;
    let panic = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let enabled = panic.clone();
    let udf = create_udf(
        "may_panic",
        vec![DataType::Int64],
        DataType::Int64,
        Volatility::Immutable,
        Arc::new(move |args| {
            assert!(!enabled.load(Ordering::SeqCst), "test producer panic");
            Ok(args[0].clone())
        }),
    );
    let source = Arc::new(ControlledSource::new(&common::snapshot(&[1, 2, 3]))?);
    source.gated.store(true, Ordering::SeqCst);
    let mut b = crate::DataflowBuilder::new();
    let node = b.add_plan(
        "source",
        LogicalPlanBuilder::from(source.plan()?)
            .project(vec![udf.call(vec![col("value")]).alias("value")])?
            .build()?,
    )?;
    let out = b.table_output("source", &node)?;
    let p = runtime.prepare(&b.finish()?).await?;
    let first = query(&p, out);
    entered(&source).await;
    let second = query(&p, out);
    watching(&p, out, 2).await;
    source.release.notify_one();
    for task in [first, second] {
        assert!(task
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("without a result"));
    }
    idle(&runtime).await;
    assert_eq!(source.scans.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.cache_stats().entries, 0);
    panic.store(false, Ordering::SeqCst);
    source.gated.store(false, Ordering::SeqCst);
    assert_eq!(query(&p, out).await.unwrap()?.table(&out)?.num_rows(), 3);
    assert_eq!(source.scans.load(Ordering::SeqCst), 2);
    idle(&runtime).await;
    Ok(())
}
