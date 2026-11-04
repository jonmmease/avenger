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
