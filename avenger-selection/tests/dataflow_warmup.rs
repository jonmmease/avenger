mod common;
#[path = "../examples/support/composition.rs"]
mod composition;

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use async_trait::async_trait;
use avenger_datafusion_dataflow::{CachePolicy, DataflowBuilder, Runtime, RuntimeConfig};
use avenger_selection::*;
use common::*;
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::{Session, TableProvider},
    common::Result as DFResult,
    datasource::{provider_as_source, MemTable},
    functions_aggregate::expr_fn::count,
    logical_expr::{col, lit, Expr, LogicalPlan, LogicalPlanBuilder, TableType},
    physical_plan::ExecutionPlan,
    prelude::SessionContext,
};
use tokio::sync::{oneshot, Notify};

const DEADLINE: Duration = Duration::from_secs(10);

#[derive(Debug)]
struct GatedSource {
    table: MemTable,
    scans: AtomicUsize,
    cancelled_scans: AtomicUsize,
    entered: Notify,
    release: Notify,
}

struct PendingScan<'a>(Option<&'a AtomicUsize>);
impl Drop for PendingScan<'_> {
    fn drop(&mut self) {
        if let Some(cancelled) = self.0 {
            cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[async_trait]
impl TableProvider for GatedSource {
    fn schema(&self) -> SchemaRef {
        self.table.schema()
    }
    fn table_type(&self) -> TableType {
        TableType::Base
    }
    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DFResult<Arc<dyn ExecutionPlan>> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        let mut pending = PendingScan(Some(&self.cancelled_scans));
        self.entered.notify_one();
        self.release.notified().await;
        // Only dropping a scan before the gate opens counts as cancellation.
        pending.0 = None;
        self.table.scan(state, projection, filters, limit).await
    }
}

type IdleSignal = Mutex<Option<oneshot::Sender<()>>>;

async fn until_scheduler_idle(signal: &IdleSignal) {
    let (send, receive) = oneshot::channel();
    assert!(signal.lock().unwrap().replace(send).is_none());
    tokio::time::timeout(DEADLINE, receive)
        .await
        .expect("scheduler reaches the gated wait")
        .expect("scheduler signals before parking");
}

fn measures_plan(rows: LogicalPlan, moments: bool) -> DFResult<LogicalPlan> {
    LogicalPlanBuilder::from(rows)
        .aggregate(
            vec![col("carrier")],
            if moments {
                measures("distance")
            } else {
                vec![count(lit(1_i64)).alias("n")]
            },
        )?
        .sort(vec![col("carrier").sort(true, true)])?
        .build()
}

#[test]
fn brush_query_shares_pending_materialization_after_warmup_is_cancelled(
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let idle = Arc::new(IdleSignal::new(None));
    let on_park = idle.clone();
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .on_thread_park(move || {
            if let Some(send) = on_park.lock().unwrap().take() {
                let _ = send.send(());
            }
        })
        .build()?;
    executor.block_on(async {
        let focus = interval("delay", "delay");
        let inactive = state(Resolution::Intersect);
        let brushed = inactive.set(&focus, between("delay", 11, 30))?;
        for moments in [false, true] {
            let context = SessionContext::new();
            let direct = measures_plan(
                context
                    .read_batch(flights())?
                    .filter(membership().predicate(&brushed)?)?
                    .into_unoptimized_plan(),
                moments,
            )?;
            let expected = context
                .execute_logical_plan(direct)
                .await?
                .collect()
                .await?;

            for cache in [CachePolicy::default(), CachePolicy::Disabled] {
                let retained = !matches!(cache, CachePolicy::Disabled);
                let source = Arc::new(GatedSource {
                    table: MemTable::try_new(flights().schema(), vec![vec![flights()]])?,
                    scans: AtomicUsize::new(0),
                    cancelled_scans: AtomicUsize::new(0),
                    entered: Notify::new(),
                    release: Notify::new(),
                });
                // Keep the scan inside the generated materialization. A second build
                // must enter a second scan, rather than sharing a separate source node.
                let source_plan =
                    LogicalPlanBuilder::scan("flights", provider_as_source(source.clone()), None)?
                        .build()?;
                let mut graph = DataflowBuilder::new();
                let predicates = membership().predicates(&inactive, &focus)?;
                let split = predicates.split().unwrap();
                let plans =
                    composition::prepare(source_plan, split, |rows| measures_plan(rows, moments))?;
                let installed =
                    composition::OptimizedQuery::install(&mut graph, "airlines", plans, split)?
                        .unwrap();
                let runtime = Runtime::new(RuntimeConfig {
                    cache,
                    ..Default::default()
                })?;
                let prepared = runtime.prepare(&graph.finish()?).await?;
                assert_eq!(
                    source.scans.load(Ordering::SeqCst),
                    0,
                    "preparation reads no source data"
                );

                let materialization = installed.query.materialization_output().unwrap();
                let warm_inputs = installed
                    .bind(split)?
                    .unwrap()
                    .apply(prepared.inputs())?
                    .finish()?;
                let warm_prepared = prepared.clone();
                let warm_task = tokio::spawn(async move {
                    warm_prepared
                        .query(&[materialization], &[], &warm_inputs)
                        .await
                });
                tokio::time::timeout(DEADLINE, source.entered.notified())
                    .await
                    .expect("warm-up enters its materialization scan");

                let brush = membership().predicates(&brushed, &focus)?;
                let binding = installed.bind(brush.split().unwrap())?.unwrap();
                let output = binding.output();
                let brush_inputs = binding.apply(prepared.inputs())?.finish()?;
                let brush_prepared = prepared.clone();
                let brush_task = tokio::spawn(async move {
                    // Request only the final chart, so interest must reach its parent.
                    brush_prepared.query(&[output], &[], &brush_inputs).await
                });

                // On this single-thread runtime, parking means queued query jobs have
                // reached their waits. The closed scan gate is their only data wait.
                // This establishes overlap without sleeps or private waiter counters.
                until_scheduler_idle(&idle).await;
                assert!(!warm_task.is_finished());
                assert!(!brush_task.is_finished());
                assert_eq!(
                    source.scans.load(Ordering::SeqCst),
                    1,
                    "both requests share one materialization"
                );
                assert_eq!(
                    runtime.cache_stats().entries,
                    0,
                    "the materialization has not completed"
                );

                warm_task.abort();
                assert!(tokio::time::timeout(DEADLINE, warm_task)
                    .await?
                    .unwrap_err()
                    .is_cancelled());
                until_scheduler_idle(&idle).await;
                assert!(!brush_task.is_finished());
                assert_eq!(
                    source.cancelled_scans.load(Ordering::SeqCst),
                    0,
                    "the brush keeps its parent's work alive"
                );
                assert_eq!(
                    source.scans.load(Ordering::SeqCst),
                    1,
                    "the parent was not restarted"
                );

                source.release.notify_one();
                let result = tokio::time::timeout(DEADLINE, brush_task).await???;
                assert_results(result.table(&output)?.batches(), &expected, FLOAT_MEASURES);
                assert_eq!(result.report().in_flight_hits, 1);
                assert_eq!(result.report().cache_hits, 0);
                assert_eq!(result.report().executed_nodes, vec!["airlines_rollup"]);
                assert_eq!(source.scans.load(Ordering::SeqCst), 1);
                assert_eq!(source.cancelled_scans.load(Ordering::SeqCst), 0);
                assert_eq!(runtime.cache_stats().entries > 0, retained);
                assert!(inactive.contributions(&id())?.next().is_none());
            }
        }
        Ok(())
    })
}
