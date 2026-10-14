use super::*;
use crate::runtime::{
    cache_aware::tests::Fixture,
    tests::{idle, until},
};
use crate::{
    CacheAwareOptions, CacheNode, CacheTargets, Dataflow, ExecutionConfig, QueryInputs, Reference,
    Result, Runtime, RuntimeConfig,
};
use std::{sync::atomic::Ordering, time::Duration};

#[tokio::test]
async fn replacing_pending_requests_preserves_group_position_and_bounds_backlog() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let first = Fixture::new(&runtime).await?;
    let second = Fixture::new(&runtime).await?;
    first.source.gated.store(true, Ordering::SeqCst);
    second.source.gated.store(true, Ordering::SeqCst);
    let permit = runtime.inner.queries.acquire_many(4).await.unwrap();
    let mut current = first.query(1, 0, true)?;
    let other = second.query(1, 0, true)?;
    for version in 2..=1000 {
        current = first.query(version, 0, true)?;
    }
    {
        let state = runtime.inner.warming.state.lock().unwrap();
        assert_eq!(state.groups.len(), 2);
        assert_eq!(state.groups[0].namespace, first.flow.inner.namespace);
        assert_eq!(state.groups[0].requests.len(), 1);
        assert_eq!(state.groups[1].requests.len(), 1);
    }
    drop(permit);
    first.entered().await;
    assert_eq!(second.source.scans.load(Ordering::SeqCst), 0);
    drop(current);
    first.source.release.notify_one();
    second.entered().await;
    first.warmed(1000).await;
    assert_eq!(first.source.scans.load(Ordering::SeqCst), 1);
    second.source.release.notify_one();
    second.warmed(1).await;
    until(|| runtime.inner.warming.is_idle()).await;
    drop(other);
    Ok(())
}

#[tokio::test]
async fn groups_take_turns_and_distinct_retained_requests_each_complete() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let a = Fixture::new(&runtime).await?;
    let b = Fixture::new(&runtime).await?;
    a.source.gated.store(true, Ordering::SeqCst);
    b.source.gated.store(true, Ordering::SeqCst);
    let permit = runtime.inner.queries.acquire_many(4).await.unwrap();
    let a1 = a.query(1, 0, true)?;
    let a2 = a.query(2, 0, true)?;
    let a3 = a.query(3, 0, true)?;
    let b1 = b.query(1, 0, true)?;
    drop(permit);
    a.entered().await;
    a.source.release.notify_one();
    b.entered().await;
    assert_eq!(a.source.scans.load(Ordering::SeqCst), 1);
    b.source.release.notify_one();
    a.entered().await;
    assert_eq!(a.source.scans.load(Ordering::SeqCst), 2);
    a.source.release.notify_one();
    a.entered().await;
    a.source.release.notify_one();
    for version in 1..=3 {
        a.warmed(version).await;
    }
    b.warmed(1).await;
    until(|| runtime.inner.warming.is_idle()).await;
    drop((a1, a2, a3, b1));
    Ok(())
}

#[tokio::test]
async fn grouping_uses_canonical_target_sets_and_preparation_identity() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let f = Fixture::new(&runtime).await?;
    let second = runtime
        .prepare(&Dataflow {
            inner: f.flow.inner.graph.clone(),
        })
        .await?;
    let permit = runtime.inner.queries.acquire_many(4).await.unwrap();
    let named = |name: &str| {
        CacheNode::Named(Reference {
            scope: vec![],
            name: name.into(),
        })
    };
    let query = |targets| {
        f.flow.cache_aware_query(
            &[f.output],
            &[],
            QueryInputs::new(f.inputs(1, 0)?),
            CacheAwareOptions {
                targets: CacheTargets::Nodes(targets),
                start_latest: true,
            },
        )
    };
    let first = query(vec![CacheNode::Plan(f.target.clone()), named("raw")])?;
    let equivalent = query(vec![named("raw"), named("total"), named("total")])?;
    let separate_targets = f.query(1, 0, true)?;
    let default_targets = f.flow.cache_aware_query(
        &[f.total],
        &[],
        QueryInputs::new(f.inputs(1, 9)?),
        CacheAwareOptions {
            start_latest: true,
            ..Default::default()
        },
    )?;
    let separate_preparation = second.cache_aware_query(
        &[f.total],
        &[],
        QueryInputs::new(f.inputs(1, 0)?),
        CacheAwareOptions {
            start_latest: true,
            ..Default::default()
        },
    )?;
    {
        let state = runtime.inner.warming.state.lock().unwrap();
        assert_eq!(state.groups.len(), 3);
        assert_eq!(state.groups[0].requests.len(), 2);
        assert_eq!(state.groups[1].requests.len(), 2);
        assert_eq!(state.groups[2].requests.len(), 1);
        assert_ne!(state.groups[0].namespace, state.groups[2].namespace);
    }
    drop((
        first,
        equivalent,
        separate_targets,
        default_targets,
        separate_preparation,
    ));
    until(|| runtime.inner.warming.is_idle()).await;
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 0);
    drop(permit);
    Ok(())
}

#[tokio::test]
async fn updates_preserve_the_dispatchers_execution_semaphore_position() -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig {
        execution: ExecutionConfig {
            max_active_queries: 1,
            ..Default::default()
        },
        ..Default::default()
    })?;
    let f = Fixture::new(&runtime).await?;
    f.source.gated.store(true, Ordering::SeqCst);
    let permit = runtime.inner.queries.acquire().await.unwrap();
    // Poll the actual dispatcher directly to establish semaphore order without timing assumptions.
    runtime.inner.warming.state.lock().unwrap().dispatching = true;
    let dispatcher = Dispatcher {
        runtime: runtime.inner.clone(),
        armed: true,
    }
    .run();
    tokio::pin!(dispatcher);
    let mut current = f.query(1, 0, true)?;
    assert!(futures::poll!(&mut dispatcher).is_pending());
    let foreground = runtime.inner.queries.acquire();
    tokio::pin!(foreground);
    assert!(futures::poll!(&mut foreground).is_pending());
    for version in 2..=10 {
        current = f.query(version, 0, true)?;
        assert!(futures::poll!(&mut dispatcher).is_pending());
    }
    drop(permit);
    assert!(futures::poll!(&mut dispatcher).is_pending());
    assert!(futures::poll!(&mut foreground).is_pending());
    f.entered().await;
    drop(current);
    f.source.release.notify_one();
    tokio::time::timeout(Duration::from_secs(10), dispatcher)
        .await
        .unwrap();
    f.warmed(10).await;
    drop(foreground.await.unwrap());
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn dropping_the_last_pending_request_releases_inputs_and_preparation() -> Result<()> {
    use crate::{
        arrow::{array::Int64Array, record_batch::RecordBatch},
        DataflowBuilder, TableSnapshot,
    };
    let runtime = Runtime::new(Default::default())?;
    let weak_runtime = Arc::downgrade(&runtime.inner);
    let permit = runtime.inner.queries.acquire_many(4).await.unwrap();
    let values = Arc::new(Int64Array::from(vec![1, 2, 3]));
    let weak_values = Arc::downgrade(&values);
    let schema = crate::runtime::tests::common::schema();
    let snapshot = TableSnapshot::from_batches(
        schema.clone(),
        vec![RecordBatch::try_new(schema.clone(), vec![values])?],
    )?;
    let mut builder = DataflowBuilder::new();
    let input = builder.table_input("source", schema)?;
    let node = builder.add_plan("source", input.plan_ref())?;
    let output = builder.table_output("source", &node)?;
    let flow = runtime.prepare(&builder.finish()?).await?;
    let weak_preparation = Arc::downgrade(&flow.inner);
    let query = flow.cache_aware_query(
        &[output],
        &[],
        QueryInputs::new(flow.inputs().table(&input, snapshot)?.finish()?),
        CacheAwareOptions {
            start_latest: true,
            ..Default::default()
        },
    )?;
    drop(flow);
    assert!(weak_values.upgrade().is_some());
    assert!(weak_preparation.upgrade().is_some());
    drop(query);
    assert!(weak_values.upgrade().is_none());
    assert!(weak_preparation.upgrade().is_none());
    until(|| runtime.inner.warming.is_idle()).await;
    drop(permit);
    drop(runtime);
    until(|| weak_runtime.upgrade().is_none()).await;
    Ok(())
}

#[test]
fn a_new_request_restarts_dispatch_after_executor_shutdown() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let first_executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let f = first_executor.block_on(Fixture::new(&runtime))?;
    let old = {
        let _entered = first_executor.enter();
        f.query(1, 0, true)?
    };
    assert!(runtime.inner.warming.state.lock().unwrap().dispatching);
    drop(first_executor);
    assert!(!runtime.inner.warming.state.lock().unwrap().dispatching);
    let second_executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    second_executor.block_on(async {
        let new = f.query(2, 0, true)?;
        f.warmed(1).await;
        f.warmed(2).await;
        until(|| runtime.inner.warming.is_idle()).await;
        drop((old, new));
        let next = f.query(3, 0, true)?;
        f.warmed(3).await;
        until(|| runtime.inner.warming.is_idle()).await;
        drop(next);
        idle(&runtime).await;
        Ok::<_, crate::Error>(())
    })?;
    Ok(())
}
