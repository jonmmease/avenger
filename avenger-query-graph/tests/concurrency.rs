//! Independent nodes compute at the same time, runs share computations, and a computation that no
//! run waits for is taken over or stops.

mod common;

use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use avenger_query_graph::{Inputs, QueryGraph, ResultStore};
use common::{exact, inputs, outcomes, state};
use datafusion::{
    arrow::{
        array::{Float64Array, RecordBatch, StringArray},
        datatypes::{DataType, Field, Schema, SchemaRef},
    },
    catalog::streaming::StreamingTable,
    execution::TaskContext,
    physical_plan::{
        stream::RecordBatchStreamAdapter, streaming::PartitionStream, SendableRecordBatchStream,
    },
    prelude::SessionContext,
    scalar::ScalarValue,
};
use tokio::sync::{watch, Barrier};

/// What holds a gated table's scans: a barrier that two scans must reach together, or tickets that
/// the test releases.
#[derive(Debug, Clone)]
enum Gate {
    Barrier(Arc<Barrier>),
    Tickets(watch::Sender<BTreeSet<usize>>),
}

/// A table whose scans wait at a gate, and that counts its scans and the scans that stopped
/// before they finished.
#[derive(Debug)]
struct Gated {
    schema: SchemaRef,
    batch: RecordBatch,
    gate: Gate,
    scans: Arc<AtomicUsize>,
    stopped: Arc<Mutex<BTreeSet<usize>>>,
}

/// Records that a scan stopped unless it finished.
struct Watch {
    ticket: usize,
    finished: bool,
    stopped: Arc<Mutex<BTreeSet<usize>>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        if !self.finished {
            self.stopped.lock().unwrap().insert(self.ticket);
        }
    }
}

impl PartitionStream for Gated {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    fn execute(&self, _ctx: Arc<TaskContext>) -> SendableRecordBatchStream {
        let ticket = self.scans.fetch_add(1, Ordering::SeqCst) + 1;
        let gate = self.gate.clone();
        let batch = self.batch.clone();
        let watch = Watch {
            ticket,
            finished: false,
            stopped: self.stopped.clone(),
        };
        let stream = futures::stream::once(async move {
            // The whole guard moves into the scan, so it drops when the scan stops.
            let mut watch = watch;
            match gate {
                Gate::Barrier(barrier) => {
                    barrier.wait().await;
                }
                Gate::Tickets(tickets) => {
                    let mut receiver = tickets.subscribe();
                    receiver
                        .wait_for(|released| released.contains(&ticket))
                        .await
                        .unwrap();
                }
            }
            watch.finished = true;
            Ok(batch)
        });
        Box::pin(RecordBatchStreamAdapter::new(self.schema.clone(), stream))
    }
}

/// A gated table of a key `k` and a number `x`, registered as `name`, with its scan count and the
/// tickets of the scans that stopped.
fn register(
    ctx: &SessionContext,
    name: &str,
    gate: Gate,
) -> (Arc<AtomicUsize>, Arc<Mutex<BTreeSet<usize>>>) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("k", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["a", "b", "a", "c", "b", "a"])),
            Arc::new(Float64Array::from(vec![1.0, 5.0, 3.0, 8.0, 2.0, 9.0])),
        ],
    )
    .unwrap();
    let scans = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(Mutex::new(BTreeSet::new()));
    let partition = Arc::new(Gated {
        schema: schema.clone(),
        batch,
        gate,
        scans: scans.clone(),
        stopped: stopped.clone(),
    });
    let table = StreamingTable::try_new(schema, vec![partition]).unwrap();
    ctx.register_table(name, Arc::new(table)).unwrap();
    (scans, stopped)
}

async fn graph(ctx: &SessionContext, store: &ResultStore, sql: &str) -> QueryGraph {
    store
        .compile(&ctx.state(), vec![common::plan(ctx, sql).await])
        .unwrap()
}

/// Wait until `condition` holds, or fail after five seconds.
async fn until(condition: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the condition holds within five seconds");
}

fn spawn(graph: &QueryGraph, inputs: Inputs) -> tokio::task::JoinHandle<avenger_query_graph::Run> {
    let graph = graph.clone();
    tokio::spawn(async move { graph.run(&inputs).await.unwrap() })
}

fn number(name: &str, value: f64) -> (&str, ScalarValue) {
    (name, ScalarValue::from(value))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_branches_compute_at_the_same_time() {
    let ctx = common::context();
    let barrier = Arc::new(Barrier::new(2));
    register(&ctx, "a", Gate::Barrier(barrier.clone()));
    register(&ctx, "b", Gate::Barrier(barrier));
    let store = common::store();
    // Each branch waits until the other has started, so the run finishes only when both compute
    // at the same time.
    let graph = graph(
        &ctx,
        &store,
        "SELECT k, count(*) AS n FROM a WHERE x > $p GROUP BY k \
         UNION ALL SELECT k, count(*) AS n FROM b WHERE x > $q GROUP BY k",
    )
    .await;
    let values = inputs(&[number("p", 2.0), number("q", 3.0)], &[]);
    let run = tokio::time::timeout(Duration::from_secs(5), graph.run(&values))
        .await
        .expect("the branches compute at the same time")
        .unwrap();
    assert_eq!(outcomes(&run), ["computed", "computed", "computed"]);
}

/// A plan whose lower node filters the gated table `a` and whose top node reads `$p`.
const LAYERED: &str = "SELECT k, count(*) AS n FROM (SELECT * FROM a WHERE x > 0) \
                       WHERE x > $p GROUP BY k";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_runs_share_one_computation() {
    let ctx = common::context();
    let (tickets, _) = watch::channel(BTreeSet::new());
    let (scans, _) = register(&ctx, "a", Gate::Tickets(tickets.clone()));
    let store = common::store();
    let graph = graph(&ctx, &store, LAYERED).await;
    let values = inputs(&[number("p", 2.0)], &[]);
    let first = spawn(&graph, values.clone());
    until(|| scans.load(Ordering::SeqCst) == 1).await;
    // The second run starts while the first run's top node still waits for the node below it.
    let second = spawn(&graph, values);
    tokio::time::sleep(Duration::from_millis(50)).await;
    tickets.send_modify(|released| {
        released.insert(1);
    });
    let (first, second) = (first.await.unwrap(), second.await.unwrap());
    assert_eq!(scans.load(Ordering::SeqCst), 1);
    assert_eq!(first.batches, second.batches);
    // The second run waits for the first run's top node, so it reads nothing below it.
    assert_eq!(outcomes(&second), ["not read", "computed"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_that_needs_another_column_starts_a_wider_computation() {
    let ctx = common::context();
    let (tickets, _) = watch::channel(BTreeSet::new());
    let (scans, _) = register(&ctx, "a", Gate::Tickets(tickets.clone()));
    let store = common::store();
    // The lower node keeps only `k` when the brush is empty, and `k` and `x` when it tests `x`.
    let graph = graph(
        &ctx,
        &store,
        "SELECT k, count(*) AS n FROM (SELECT * FROM a WHERE k <> 'z') \
         WHERE selected($s) GROUP BY k",
    )
    .await;
    let empty = inputs(&[], &[("s", avenger_selection::SelectionState::default())]);
    let boxed = inputs(&[], &[("s", state(vec![("x", exact("x", 2.0, 8.0))]))]);
    let narrow = spawn(&graph, empty.clone());
    until(|| scans.load(Ordering::SeqCst) == 1).await;
    let wide = spawn(&graph, boxed.clone());
    until(|| scans.load(Ordering::SeqCst) == 2).await;
    // The wider computation finishes first, and the narrower one finishes later.
    tickets.send_modify(|released| {
        released.insert(2);
    });
    wide.await.unwrap();
    tickets.send_modify(|released| {
        released.insert(1);
    });
    narrow.await.unwrap();
    // The kept result is the wider one, so a run that tests `x` hits it.
    tickets.send_modify(|released| {
        released.extend(3..10);
    });
    let other = inputs(&[], &[("s", state(vec![("x", exact("x", 1.0, 4.0))]))]);
    let run = graph.run(&other).await.unwrap();
    assert_eq!(outcomes(&run), ["hit", "computed"]);
    assert_eq!(scans.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_next_run_takes_over_a_computation_that_it_needs() {
    let ctx = common::context();
    let (tickets, _) = watch::channel(BTreeSet::new());
    let (scans, stopped) = register(&ctx, "a", Gate::Tickets(tickets.clone()));
    let store = common::store();
    let graph = graph(&ctx, &store, LAYERED).await;
    let dropped = spawn(&graph, inputs(&[number("p", 2.0)], &[]));
    until(|| scans.load(Ordering::SeqCst) == 1).await;
    dropped.abort();
    let _ = dropped.await;
    // The next run reads the same lower node for another value of `$p`.
    let next = spawn(&graph, inputs(&[number("p", 4.0)], &[]));
    tokio::time::sleep(Duration::from_millis(50)).await;
    tickets.send_modify(|released| {
        released.insert(1);
    });
    let run = next.await.unwrap();
    assert_eq!(outcomes(&run), ["computed", "computed"]);
    assert_eq!(scans.load(Ordering::SeqCst), 1);
    assert!(stopped.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_next_run_stops_a_computation_that_it_does_not_need() {
    let ctx = common::context();
    let (tickets, _) = watch::channel(BTreeSet::new());
    let (scans, stopped) = register(&ctx, "a", Gate::Tickets(tickets.clone()));
    let store = common::store();
    let graph = graph(
        &ctx,
        &store,
        "SELECT k, count(*) AS n FROM (SELECT * FROM a WHERE x > $q) \
         WHERE k <> $p GROUP BY k",
    )
    .await;
    let values = |q: f64| inputs(&[number("q", q), ("p", ScalarValue::from("z"))], &[]);
    let dropped = spawn(&graph, values(1.0));
    until(|| scans.load(Ordering::SeqCst) == 1).await;
    dropped.abort();
    let _ = dropped.await;
    // The next run filters by another `$q`, so it does not need the dropped run's lower node.
    let next = spawn(&graph, values(3.0));
    until(|| scans.load(Ordering::SeqCst) == 2).await;
    until(|| stopped.lock().unwrap().contains(&1)).await;
    tickets.send_modify(|released| {
        released.insert(2);
    });
    let run = next.await.unwrap();
    assert_eq!(outcomes(&run), ["computed", "computed"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_with_the_same_inputs_takes_over_the_whole_computation() {
    let ctx = common::context();
    let (tickets, _) = watch::channel(BTreeSet::new());
    let (scans, stopped) = register(&ctx, "a", Gate::Tickets(tickets.clone()));
    let store = common::store();
    let graph = graph(&ctx, &store, LAYERED).await;
    let values = inputs(&[number("p", 2.0)], &[]);
    let dropped = spawn(&graph, values.clone());
    until(|| scans.load(Ordering::SeqCst) == 1).await;
    dropped.abort();
    let _ = dropped.await;
    // The next run joins the top node's computation and reads nothing below it, so the lower
    // node's computation continues for the top node's.
    let next = spawn(&graph, values);
    tokio::time::sleep(Duration::from_millis(50)).await;
    tickets.send_modify(|released| {
        released.insert(1);
    });
    let run = next.await.unwrap();
    assert_eq!(outcomes(&run), ["not read", "computed"]);
    assert_eq!(scans.load(Ordering::SeqCst), 1);
    assert!(stopped.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_next_run_stops_a_volatile_computation_that_no_run_waits_for() {
    let ctx = common::context();
    let (tickets, _) = watch::channel(BTreeSet::new());
    let (scans, stopped) = register(&ctx, "a", Gate::Tickets(tickets.clone()));
    let store = common::store();
    let graph = graph(&ctx, &store, "SELECT random() AS r FROM a").await;
    let dropped = spawn(&graph, Inputs::default());
    until(|| scans.load(Ordering::SeqCst) == 1).await;
    dropped.abort();
    let _ = dropped.await;
    let next = spawn(&graph, Inputs::default());
    until(|| stopped.lock().unwrap().contains(&1)).await;
    tickets.send_modify(|released| {
        released.extend([1, 2]);
    });
    let run = next.await.unwrap();
    assert_eq!(outcomes(&run), ["uncached"]);
}

/// Start a run in a current-thread runtime, drop it after it spawns its computation, after
/// `yields` yields, and drop the runtime, then run the graph again in another runtime.
fn rerun_after_a_dropped_runtime(yields: usize) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let graph = runtime.block_on(async {
        let ctx = SessionContext::new();
        let graph = common::store()
            .compile(
                &ctx.state(),
                vec![common::plan(&ctx, "SELECT 1 AS n").await],
            )
            .unwrap();
        {
            let inputs = Inputs::default();
            let run = graph.run(&inputs);
            futures::pin_mut!(run);
            // The run spawns its computation and waits for it, and the runtime has not polled
            // the computation's task yet.
            assert!(futures::poll!(run.as_mut()).is_pending());
            for _ in 0..yields {
                tokio::task::yield_now().await;
            }
        }
        graph
    });
    drop(runtime);
    let next = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    next.block_on(graph.run(&Inputs::default())).unwrap();
}

#[test]
fn a_computation_whose_task_never_ran_leaves_the_store() {
    rerun_after_a_dropped_runtime(0);
}

#[test]
fn a_computation_whose_task_started_leaves_the_store() {
    rerun_after_a_dropped_runtime(1);
}

/// An optimizer rule that pauses its first rewrite until the test releases it, after it tells the
/// test that it paused.
#[derive(Debug)]
struct PausesOnce {
    gate: std::sync::Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
}

impl datafusion::optimizer::OptimizerRule for PausesOnce {
    fn name(&self) -> &str {
        "pauses_once"
    }

    fn rewrite(
        &self,
        plan: datafusion::logical_expr::LogicalPlan,
        _: &dyn datafusion::optimizer::OptimizerConfig,
    ) -> datafusion::common::Result<
        datafusion::common::tree_node::Transformed<datafusion::logical_expr::LogicalPlan>,
    > {
        if let Some((paused, release)) = self.gate.lock().unwrap().take() {
            paused.send(()).unwrap();
            release.recv().unwrap();
        }
        Ok(datafusion::common::tree_node::Transformed::no(plan))
    }
}

/// Sends when it is dropped.
struct Dropped(std::sync::mpsc::Sender<()>);

impl Drop for Dropped {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// Run a graph of two nodes on a runtime of two workers, and pause the run while it optimizes,
/// after it spawned its first computation. When `shutdown`, the runtime begins to shut down
/// during the pause, so the run's next spawn is rejected.
fn run_through_a_pause(shutdown: bool) {
    use std::sync::mpsc;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (paused, wait) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let graph = runtime.block_on(async {
        let ctx = common::context();
        let plan = common::plan(
            &ctx,
            "SELECT carrier, count(*) AS n FROM (SELECT * FROM flights WHERE delay > 0) \
             WHERE year = $year GROUP BY carrier",
        )
        .await;
        let state = datafusion::execution::session_state::SessionStateBuilder::new_from_existing(
            ctx.state(),
        )
        .with_optimizer_rule(Arc::new(PausesOnce {
            gate: std::sync::Mutex::new(Some((paused, released))),
        }))
        .build();
        let graph = common::store().compile(&state, vec![plan]).unwrap();
        assert_eq!(graph.nodes(None).unwrap().len(), 2);
        graph
    });
    // A pending task drops only once a worker has closed the runtime's tasks.
    let (closed, closing) = mpsc::channel();
    let sentinel = Dropped(closed);
    runtime.spawn(async move {
        let _sentinel = sentinel;
        futures::future::pending::<()>().await;
    });
    let (finished, finishing) = mpsc::channel();
    runtime.spawn(async move {
        let values = inputs(&[("year", ScalarValue::from(2023i64))], &[]);
        let _ = finished.send(graph.run(&values).await);
    });
    wait.recv_timeout(Duration::from_secs(5)).unwrap();
    let runtime = match shutdown {
        true => {
            runtime.shutdown_background();
            closing.recv_timeout(Duration::from_secs(5)).unwrap();
            None
        }
        false => Some(runtime),
    };
    release.send(()).unwrap();
    let result = finishing.recv_timeout(Duration::from_secs(2));
    if let Some(runtime) = runtime {
        runtime.shutdown_background();
    }
    match shutdown {
        // The run leaves its poll: it returns, or the runtime cancels it.
        true => assert!(!matches!(result, Err(mpsc::RecvTimeoutError::Timeout))),
        false => {
            result.unwrap().unwrap();
        }
    }
}

#[test]
fn a_run_finishes_on_a_running_runtime() {
    run_through_a_pause(false);
}

#[test]
fn a_spawn_that_shutdown_rejects_does_not_deadlock_the_run() {
    run_through_a_pause(true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_result_of_a_table_that_changed_while_it_computed_is_not_kept() {
    let ctx = common::context();
    let (tickets, _) = watch::channel(BTreeSet::new());
    let (scans, _) = register(&ctx, "a", Gate::Tickets(tickets.clone()));
    let store = common::store();
    let graph = graph(&ctx, &store, "SELECT k, count(*) AS n FROM a GROUP BY k").await;
    let first = spawn(&graph, Inputs::default());
    until(|| scans.load(Ordering::SeqCst) == 1).await;
    store.table_changed("a");
    tickets.send_modify(|released| {
        released.extend(1..10);
    });
    assert_eq!(outcomes(&first.await.unwrap()), ["uncached"]);
    let next = graph.run(&Inputs::default()).await.unwrap();
    assert_eq!(outcomes(&next), ["computed"]);
    assert_eq!(
        outcomes(&graph.run(&Inputs::default()).await.unwrap()),
        ["hit"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_a_graph_stops_its_computations_that_nothing_waits_for() {
    let ctx = common::context();
    let (tickets, _) = watch::channel(BTreeSet::new());
    let (scans, stopped) = register(&ctx, "a", Gate::Tickets(tickets.clone()));
    let store = common::store();
    let old = graph(&ctx, &store, LAYERED).await;
    let dropped = spawn(&old, inputs(&[number("p", 2.0)], &[]));
    until(|| scans.load(Ordering::SeqCst) == 1).await;
    dropped.abort();
    let _ = dropped.await;
    // A reload drops the old graph, and its computation that nothing waits for stops.
    drop(old);
    until(|| stopped.lock().unwrap().contains(&1)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dropped_graphs_computation_that_a_new_graph_joined_continues() {
    let ctx = common::context();
    let (tickets, _) = watch::channel(BTreeSet::new());
    let (scans, stopped) = register(&ctx, "a", Gate::Tickets(tickets.clone()));
    let store = common::store();
    let old = graph(&ctx, &store, LAYERED).await;
    let dropped = spawn(&old, inputs(&[number("p", 2.0)], &[]));
    until(|| scans.load(Ordering::SeqCst) == 1).await;
    // The reloaded plan has a new top over the old plan's lower node, and its run joins the
    // lower node's computation before the old graph drops.
    let new = graph(
        &ctx,
        &store,
        "SELECT k, max(x) AS m FROM (SELECT * FROM a WHERE x > 0) WHERE x > $p GROUP BY k",
    )
    .await;
    let next = spawn(&new, inputs(&[number("p", 2.0)], &[]));
    tokio::time::sleep(Duration::from_millis(50)).await;
    dropped.abort();
    let _ = dropped.await;
    drop(old);
    tickets.send_modify(|released| {
        released.insert(1);
    });
    let run = next.await.unwrap();
    assert_eq!(outcomes(&run), ["computed", "computed"]);
    assert_eq!(scans.load(Ordering::SeqCst), 1);
    assert!(stopped.lock().unwrap().is_empty());
}
