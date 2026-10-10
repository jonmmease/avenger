use super::*;
use async_trait::async_trait;
use datafusion::{
    catalog::{Session, TableProvider},
    common::Result as DFResult,
    datasource::provider_as_source,
    logical_expr::{Expr, LogicalPlanBuilder, TableType},
    physical_expr::EquivalenceProperties,
    physical_plan::{
        coalesce_partitions::CoalescePartitionsExec,
        execution_plan::{Boundedness, EmissionType},
        stream::RecordBatchStreamAdapter,
        DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
        SendableRecordBatchStream,
    },
};
use std::{
    fmt,
    sync::{atomic::AtomicBool, Condvar},
    time::Duration,
};
use tokio::sync::Notify;

#[derive(Debug, Default)]
struct Gate {
    entered: Notify,
    released: Mutex<bool>,
    condition: Condvar,
    enabled: AtomicBool,
    executions: AtomicUsize,
    streams: AtomicUsize,
}
impl Gate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.enabled.store(false, Ordering::SeqCst);
        self.condition.notify_all();
    }
}
struct Probe(Arc<Gate>);
impl Drop for Probe {
    fn drop(&mut self) {
        self.0.streams.fetch_sub(1, Ordering::SeqCst);
    }
}
#[derive(Debug)]
struct Source(Arc<Gate>);
#[async_trait]
impl TableProvider for Source {
    fn schema(&self) -> SchemaRef {
        super::tests::common::schema()
    }
    fn table_type(&self) -> TableType {
        TableType::Base
    }
    async fn scan(
        &self,
        _: &dyn Session,
        _: Option<&Vec<usize>>,
        _: &[Expr],
        _: Option<usize>,
    ) -> DFResult<Arc<dyn ExecutionPlan>> {
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(self.schema()),
            Partitioning::UnknownPartitioning(2),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        Ok(Arc::new(CoalescePartitionsExec::new(Arc::new(GatedExec {
            gate: self.0.clone(),
            properties,
        }))))
    }
}
#[derive(Debug)]
struct GatedExec {
    gate: Arc<Gate>,
    properties: Arc<PlanProperties>,
}
impl DisplayAs for GatedExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GatedExec")
    }
}
impl ExecutionPlan for GatedExec {
    fn name(&self) -> &str {
        "GatedExec"
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }
    fn with_new_children(
        self: Arc<Self>,
        _: Vec<Arc<dyn ExecutionPlan>>,
    ) -> DFResult<Arc<dyn ExecutionPlan>> {
        Ok(self)
    }
    fn execute(
        &self,
        partition: usize,
        _: Arc<datafusion::execution::TaskContext>,
    ) -> DFResult<SendableRecordBatchStream> {
        self.gate.executions.fetch_add(1, Ordering::SeqCst);
        self.gate.streams.fetch_add(1, Ordering::SeqCst);
        let probe = Probe(self.gate.clone());
        let stream = futures::stream::once(async move {
            if partition == 0 && probe.0.enabled.load(Ordering::SeqCst) {
                probe.0.entered.notify_one();
                let (_guard, timeout) = probe
                    .0
                    .condition
                    .wait_timeout_while(
                        probe.0.released.lock().unwrap(),
                        Duration::from_secs(10),
                        |released| !*released,
                    )
                    .unwrap();
                assert!(!timeout.timed_out(), "test releases synchronous kernel");
            }
            let batch = super::tests::common::snapshot(&[partition as i64]).batches()[0].clone();
            drop(probe);
            Ok(batch)
        });
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            self.schema(),
            stream,
        )))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_waits_for_datafusion_worker_teardown_before_replacement_and_budget_release(
) -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig {
        cache: crate::CachePolicy::Disabled,
        ..Default::default()
    })?;
    let gate = Arc::new(Gate::default());
    gate.enabled.store(true, Ordering::SeqCst);
    let mut b = crate::DataflowBuilder::new();
    let plan = LogicalPlanBuilder::scan(
        "source",
        provider_as_source(Arc::new(Source(gate.clone()))),
        None,
    )?
    .build()?;
    let node = b.add_plan("source", plan)?;
    let out = b.table_output("source", &node)?;
    let p = runtime.prepare(&b.finish()?).await?;
    let first = super::tests::query(&p, out);
    gate.entered.notified().await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    super::tests::until(|| runtime.inner.cache.lock().unwrap().stopping()).await;
    assert!(runtime.inner.active_bytes.load(Ordering::Relaxed) > 0);
    let executions = gate.executions.load(Ordering::SeqCst);
    let next = super::tests::query(&p, out);
    super::tests::watching(&p, out, 1).await;
    assert_eq!(gate.executions.load(Ordering::SeqCst), executions);
    assert!(!next.is_finished());
    gate.release();
    let result = tokio::time::timeout(Duration::from_secs(10), next)
        .await
        .unwrap()
        .unwrap()?;
    assert_eq!(result.table(&out)?.num_rows(), 2);
    assert_eq!(gate.executions.load(Ordering::SeqCst), executions + 2);
    assert_eq!(gate.streams.load(Ordering::SeqCst), 0);
    super::tests::idle(&runtime).await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn warming_slot_stays_occupied_until_synchronous_worker_teardown() -> Result<()> {
    use crate::{CacheAwareOptions, QueryInputs};
    let runtime = Runtime::new(Default::default())?;
    let first_gate = Arc::new(Gate::default());
    first_gate.enabled.store(true, Ordering::SeqCst);
    let second_gate = Arc::new(Gate::default());
    let mut queries = Vec::new();
    for gate in [&first_gate, &second_gate] {
        let mut builder = crate::DataflowBuilder::new();
        let plan = LogicalPlanBuilder::scan(
            "source",
            provider_as_source(Arc::new(Source(gate.clone()))),
            None,
        )?
        .build()?;
        let node = builder.add_plan("source", plan)?;
        let output = builder.table_output("rows", &node)?;
        let flow = runtime.prepare(&builder.finish()?).await?;
        queries.push((flow, output));
    }
    let (first, first_output) = &queries[0];
    let warming = first.cache_aware_query(
        &[*first_output],
        &[],
        QueryInputs::new(first.inputs().finish()?),
        CacheAwareOptions {
            start_latest: true,
            ..Default::default()
        },
    )?;
    first_gate.entered.notified().await;
    let (second, second_output) = &queries[1];
    let queued = second.cache_aware_query(
        &[*second_output],
        &[],
        QueryInputs::new(second.inputs().finish()?),
        CacheAwareOptions {
            start_latest: true,
            ..Default::default()
        },
    )?;
    drop(warming);
    assert!(!runtime.inner.warming.is_idle());
    assert_eq!(second_gate.executions.load(Ordering::SeqCst), 0);
    assert!(runtime.inner.active_bytes.load(Ordering::Relaxed) > 0);
    first_gate.release();
    super::tests::until(|| second_gate.executions.load(Ordering::SeqCst) == 2).await;
    super::tests::idle(&runtime).await;
    assert_eq!(first_gate.streams.load(Ordering::SeqCst), 0);
    assert_eq!(second_gate.streams.load(Ordering::SeqCst), 0);
    drop(queued);
    Ok(())
}
