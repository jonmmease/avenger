use super::*;
use crate::runtime::tests::{common, idle, until, watching};
use crate::{CacheConfig, CachePolicy, DataflowBuilder, ExecutionConfig, ScalarInput};
use common::source::ControlledSource;
use datafusion::{
    arrow::datatypes::DataType,
    functions_aggregate::expr_fn::sum,
    logical_expr::{col, Expr, LogicalPlanBuilder},
};
use std::time::Duration;

struct Fixture {
    source: Arc<ControlledSource>,
    flow: PreparedDataflow,
    version: ScalarInput,
    brush: ScalarInput,
    target: crate::PlanNode,
    total: TableOutput,
    output: TableOutput,
}

impl Fixture {
    async fn new(runtime: &Runtime) -> Result<Self> {
        let source = Arc::new(ControlledSource::new(&common::snapshot(&[1, 2, 3]))?);
        let mut b = DataflowBuilder::new();
        let version = b.scalar_input("version", DataType::Int64)?;
        let brush = b.scalar_input("brush", DataType::Int64)?;
        let raw = b.add_plan(
            "raw",
            LogicalPlanBuilder::from(source.plan()?)
                .project(vec![(col("value") * version.expr_ref()).alias("value")])?
                .build()?,
        )?;
        let target = b.add_plan(
            "total",
            LogicalPlanBuilder::from(raw.plan_ref())
                .aggregate(Vec::<Expr>::new(), vec![sum(col("value")).alias("value")])?
                .build()?,
        )?;
        let total = b.table_output("total", &target)?;
        let view = b.add_plan(
            "view",
            LogicalPlanBuilder::from(target.plan_ref())
                .filter(col("value").gt(brush.expr_ref()))?
                .build()?,
        )?;
        let output = b.table_output("view", &view)?;
        let flow = runtime.prepare(&b.finish()?).await?;
        Ok(Self {
            source,
            flow,
            version,
            brush,
            target,
            total,
            output,
        })
    }

    fn inputs(&self, version: i64, brush: i64) -> Result<Inputs> {
        self.flow
            .inputs()
            .scalar(&self.version, version.into())?
            .scalar(&self.brush, brush.into())?
            .finish()
    }

    fn query(&self, version: i64, brush: i64, start_latest: bool) -> Result<CacheAwareQuery> {
        self.flow.cache_aware_query(
            &[self.output],
            &[],
            QueryInputs::new(self.inputs(version, brush)?),
            CacheAwareOptions {
                targets: CacheTargets::Nodes(vec![CacheNode::Plan(self.target.clone())]),
                start_latest,
            },
        )
    }

    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(10), self.source.entered.notified())
            .await
            .unwrap();
    }

    async fn warmed(&self, version: i64) {
        let q = self
            .flow
            .cache_aware_query(
                &[self.total],
                &[],
                QueryInputs::new(self.inputs(version, 0).unwrap()),
                Default::default(),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match q.read(CacheRead::CachedOnly).await {
                    Ok(_) => break,
                    Err(Error::CacheMiss { .. }) => tokio::task::yield_now().await,
                    Err(error) => panic!("{error}"),
                }
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn eager_work_stops_at_targets_and_does_not_need_a_read() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let f = Fixture::new(&runtime).await?;
    f.source.gated.store(true, Ordering::SeqCst);
    let warming = f.query(1, 0, true)?;
    f.entered().await;
    assert!(matches!(
        warming.read(CacheRead::CachedOnly).await,
        Err(Error::CacheMiss { .. })
    ));
    f.source.release.notify_one();
    f.warmed(1).await;
    idle(&runtime).await;
    assert_eq!(runtime.cache_stats().entries, 2);
    assert!(matches!(
        warming.read(CacheRead::CachedOnly).await,
        Err(Error::CacheMiss { .. })
    ));
    let current = f
        .query(1, 7, false)?
        .read(CacheRead::FromCachedTargets)
        .await?;
    assert_eq!(current.result().table(&f.output)?.num_rows(), 0);
    assert_eq!(current.result().report().executed_nodes, ["view"]);
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 1);
    assert_eq!(
        warming
            .read(CacheRead::FromCachedTargets)
            .await?
            .result()
            .table(&f.output)?
            .num_rows(),
        1
    );
    f.flow.clear_results();
    assert!(matches!(
        warming.read(CacheRead::FromCachedTargets).await,
        Err(Error::CacheMiss { .. })
    ));
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn strict_hits_and_misses_bypass_the_only_execution_slot() -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig {
        execution: ExecutionConfig {
            max_active_queries: 1,
            ..Default::default()
        },
        ..Default::default()
    })?;
    let f = Fixture::new(&runtime).await?;
    f.flow.query(&[f.output], &[], &f.inputs(1, 0)?).await?;
    f.source.gated.store(true, Ordering::SeqCst);
    let flow = f.flow.clone();
    let output = f.output;
    let next = f.inputs(2, 0)?;
    let busy = tokio::spawn(async move { flow.query(&[output], &[], &next).await });
    f.entered().await;
    let hit = tokio::time::timeout(Duration::from_secs(2), async {
        f.query(1, 0, false)?.read(CacheRead::CachedOnly).await
    })
    .await
    .unwrap()?;
    assert_eq!(hit.result().report().physical_plans, 0);
    let miss = tokio::time::timeout(Duration::from_secs(2), async {
        f.query(2, 0, false)?.read(CacheRead::CachedOnly).await
    })
    .await
    .unwrap();
    assert!(matches!(miss, Err(Error::CacheMiss { .. })));
    let downstream = f.query(1, 9, false)?;
    let read = downstream.read(CacheRead::FromCachedTargets);
    tokio::pin!(read);
    assert!(futures::poll!(&mut read).is_pending());
    f.source.release.notify_one();
    busy.await.unwrap()?;
    read.await?;
    Ok(())
}

#[tokio::test]
async fn one_background_slot_leaves_room_for_brushing_across_preparations() -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig {
        execution: ExecutionConfig {
            max_active_queries: 2,
            ..Default::default()
        },
        ..Default::default()
    })?;
    let f = Fixture::new(&runtime).await?;
    let other = Fixture::new(&runtime).await?;
    f.flow.query(&[f.total], &[], &f.inputs(1, 0)?).await?;
    f.source.gated.store(true, Ordering::SeqCst);
    let active = f.query(2, 0, true)?;
    f.entered().await;
    let queued = other.query(1, 0, true)?;
    let pending = f.query(3, 0, true)?;
    let newest = f.query(4, 0, true)?;
    let fallback = f
        .flow
        .inputs()
        .scalar(&f.version, 1_i64.into())?
        .finish_overrides()?;
    let brush = f.flow.cache_aware_query(
        &[f.output],
        &[],
        QueryInputs::new(f.inputs(3, 5)?).fallbacks([fallback])?,
        CacheAwareOptions {
            targets: CacheTargets::Nodes(vec![CacheNode::Plan(f.target.clone())]),
            start_latest: false,
        },
    )?;
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        brush.read(CacheRead::FromCachedTargets),
    )
    .await
    .unwrap()?;
    assert_eq!(result.candidate_index(), 1);
    assert_eq!(common::values(result.result().table(&f.output)?), [6]);
    assert_eq!(result.result().report().executed_nodes, ["view"]);
    assert_eq!(other.source.scans.load(Ordering::SeqCst), 0);
    assert_eq!(runtime.inner.queries.available_permits(), 1);
    f.source.gated.store(false, Ordering::SeqCst);
    f.source.release.notify_one();
    f.warmed(2).await;
    f.warmed(3).await;
    f.warmed(4).await;
    other.warmed(1).await;
    idle(&runtime).await;
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 4);
    drop((active, queued, pending, newest));
    Ok(())
}

#[tokio::test]
async fn dropping_queries_cancels_queued_and_running_unshared_jobs() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let f = Fixture::new(&runtime).await?;
    f.source.gated.store(true, Ordering::SeqCst);
    let active = f.query(1, 0, true)?;
    f.entered().await;
    let queued = f.query(2, 0, true)?;
    drop(queued);
    drop(active);
    idle(&runtime).await;
    until(|| runtime.inner.warming.available_permits() == 1).await;
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 1);

    let permit = runtime.inner.queries.acquire_many(4).await.unwrap();
    let queued = f.query(3, 0, true)?;
    until(|| runtime.inner.warming.available_permits() == 0).await;
    drop(queued);
    until(|| runtime.inner.warming.available_permits() == 1).await;
    drop(permit);
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn dropping_warming_preserves_an_ordinary_subscriber() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let f = Fixture::new(&runtime).await?;
    f.source.gated.store(true, Ordering::SeqCst);
    let warming = f.query(1, 0, true)?;
    f.entered().await;
    let flow = f.flow.clone();
    let inputs = f.inputs(1, 9)?;
    let total = f.total;
    let subscriber = tokio::spawn(async move { flow.query(&[total], &[], &inputs).await });
    watching(&f.flow, f.total, 2).await;
    drop(warming);
    watching(&f.flow, f.total, 1).await;
    f.source.release.notify_one();
    subscriber.await.unwrap()?;
    idle(&runtime).await;
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn clear_after_target_acquisition_does_not_rebuild_target_or_publish_stale_output(
) -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let f = Fixture::new(&runtime).await?;
    f.flow.query(&[f.total], &[], &f.inputs(1, 0)?).await?;
    let permit = runtime.inner.queries.acquire_many(4).await.unwrap();
    let query = f.query(1, 0, false)?;
    let read = query.read(CacheRead::FromCachedTargets);
    tokio::pin!(read);
    assert!(futures::poll!(&mut read).is_pending());
    f.flow.clear_results();
    drop(permit);
    let result = read.await?;
    assert_eq!(common::values(result.result().table(&f.output)?), [6]);
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.cache_stats().entries, 0);
    assert!(matches!(
        query.read(CacheRead::FromCachedTargets).await,
        Err(Error::CacheMiss { .. })
    ));
    Ok(())
}

#[tokio::test]
async fn retained_queries_do_not_pin_completed_values_or_restart_bypassed_work() -> Result<()> {
    for cache in [
        CachePolicy::Disabled,
        CachePolicy::Lru(CacheConfig {
            max_bytes: 1,
            max_entries: 1,
        }),
    ] {
        let runtime = Runtime::new(RuntimeConfig {
            cache,
            ..Default::default()
        })?;
        let f = Fixture::new(&runtime).await?;
        f.source.gated.store(true, Ordering::SeqCst);
        let query = f.query(1, 0, true)?;
        f.entered().await;
        f.source.release.notify_one();
        idle(&runtime).await;
        until(|| runtime.inner.warming.available_permits() == 1).await;
        assert_eq!(runtime.cache_stats().entries, 0);
        assert!(matches!(
            query.read(CacheRead::CachedOnly).await,
            Err(Error::CacheMiss { .. })
        ));
        assert!(matches!(
            query.read(CacheRead::FromCachedTargets).await,
            Err(Error::CacheMiss { .. })
        ));
        assert_eq!(f.source.scans.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[tokio::test]
async fn default_targets_warm_requested_outputs_and_reads_do_not_keep_jobs_alive() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let f = Fixture::new(&runtime).await?;
    f.flow.query(&[f.output], &[], &f.inputs(1, 0)?).await?;
    f.source.gated.store(true, Ordering::SeqCst);
    let fallback = f
        .flow
        .inputs()
        .scalar(&f.version, 1_i64.into())?
        .finish_overrides()?;
    let query = f.flow.cache_aware_query(
        &[f.output],
        &[],
        QueryInputs::new(f.inputs(2, 0)?).fallbacks([fallback])?,
        CacheAwareOptions {
            start_latest: true,
            ..Default::default()
        },
    )?;
    f.entered().await;
    let result = query.read(CacheRead::CachedOnly).await?;
    assert_eq!(result.candidate_index(), 1);
    let read = query.read(CacheRead::FromCachedTargets);
    drop(read);
    assert_eq!(runtime.inner.warming.available_permits(), 0);
    drop(query);
    idle(&runtime).await;
    assert_eq!(result.result().table(&f.output)?.num_rows(), 1);

    f.source.gated.store(false, Ordering::SeqCst);
    let query = f.flow.cache_aware_query(
        &[f.output],
        &[],
        QueryInputs::new(f.inputs(2, 0)?),
        CacheAwareOptions {
            start_latest: true,
            ..Default::default()
        },
    )?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(result) = query.read(CacheRead::CachedOnly).await {
                assert_eq!(result.candidate_index(), 0);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    idle(&runtime).await;
    Ok(())
}

#[tokio::test]
async fn strict_acquisition_respects_active_byte_budget() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let f = Fixture::new(&runtime).await?;
    f.flow.query(&[f.output], &[], &f.inputs(1, 0)?).await?;
    let budget = Reservation {
        runtime: runtime.inner.clone(),
        bytes: AtomicUsize::new(0),
    };
    budget.charge(runtime.inner.config.execution.max_materialized_bytes)?;
    assert!(matches!(
        f.query(1, 0, false)?.read(CacheRead::CachedOnly).await,
        Err(Error::ResourceExhausted { .. })
    ));
    drop(budget);
    idle(&runtime).await;
    Ok(())
}

#[tokio::test]
async fn background_failure_preserves_cached_fallback_and_does_not_retry() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let source = Arc::new(ControlledSource::new(&common::snapshot(&[6]))?);
    let mut builder = DataflowBuilder::new();
    let divisor = builder.scalar_input("divisor", DataType::Int64)?;
    let node = builder.add_plan(
        "divide",
        LogicalPlanBuilder::from(source.plan()?)
            .project(vec![(col("value") / divisor.expr_ref()).alias("value")])?
            .build()?,
    )?;
    let output = builder.table_output("rows", &node)?;
    let flow = runtime.prepare(&builder.finish()?).await?;
    let old = flow.inputs().scalar(&divisor, 2_i64.into())?.finish()?;
    flow.query(&[output], &[], &old).await?;
    source.gated.store(true, Ordering::SeqCst);
    let latest = old.edit().scalar(&divisor, 0_i64.into())?.finish()?;
    let query = flow.cache_aware_query(
        &[output],
        &[],
        QueryInputs::new(latest).fallbacks([old.edit().finish_overrides()?])?,
        CacheAwareOptions {
            start_latest: true,
            ..Default::default()
        },
    )?;
    tokio::time::timeout(Duration::from_secs(10), source.entered.notified())
        .await
        .unwrap();
    let before = query.read(CacheRead::CachedOnly).await?;
    assert_eq!(before.candidate_index(), 1);
    source.release.notify_one();
    idle(&runtime).await;
    until(|| runtime.inner.warming.available_permits() == 1).await;
    assert_eq!(
        query.read(CacheRead::CachedOnly).await?.candidate_index(),
        1
    );
    assert_eq!(common::values(before.result().table(&output)?), [3]);
    assert_eq!(source.scans.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn explicit_target_eviction_is_not_hidden_by_a_cached_output() -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig {
        cache: CachePolicy::Lru(CacheConfig {
            max_entries: 1,
            max_bytes: 1024 * 1024,
        }),
        ..Default::default()
    })?;
    let f = Fixture::new(&runtime).await?;
    f.flow.query(&[f.output], &[], &f.inputs(1, 0)?).await?;
    let requested = f.flow.cache_aware_query(
        &[f.output],
        &[],
        QueryInputs::new(f.inputs(1, 0)?),
        Default::default(),
    )?;
    assert!(requested.read(CacheRead::CachedOnly).await.is_ok());
    assert!(matches!(
        f.query(1, 0, false)?.read(CacheRead::CachedOnly).await,
        Err(Error::CacheMiss { .. })
    ));
    f.source.gated.store(true, Ordering::SeqCst);
    let warming = f.query(1, 0, true)?;
    f.entered().await;
    f.source.release.notify_one();
    f.warmed(1).await;
    idle(&runtime).await;
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 2);
    assert!(matches!(
        warming.read(CacheRead::CachedOnly).await,
        Err(Error::CacheMiss { .. })
    ));
    Ok(())
}

#[tokio::test]
async fn clearing_during_warming_prevents_stale_publication() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let f = Fixture::new(&runtime).await?;
    f.source.gated.store(true, Ordering::SeqCst);
    let warming = f.query(1, 0, true)?;
    f.entered().await;
    f.flow.clear_results();
    f.source.release.notify_one();
    idle(&runtime).await;
    until(|| runtime.inner.warming.available_permits() == 1).await;
    assert_eq!(runtime.cache_stats().entries, 0);
    assert!(matches!(
        warming.read(CacheRead::FromCachedTargets).await,
        Err(Error::CacheMiss { .. })
    ));
    assert_eq!(f.source.scans.load(Ordering::SeqCst), 1);
    Ok(())
}
