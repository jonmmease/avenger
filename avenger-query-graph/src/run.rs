//! Runs: the keys of a graph's nodes for a run's inputs, the resolution of the nodes from the top
//! down, and the computation of the misses.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use avenger_common::time::{Duration, Instant};
use avenger_selection::SelectionState;
use avenger_selection_datafusion::{bind_selections, with_selection_nodes, with_selection_states};
use datafusion::{
    arrow::{array::RecordBatch, datatypes::SchemaRef},
    common::{plan_err, Column, DataFusionError, ParamValues, Result, ScalarValue},
    execution::SessionState,
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
    physical_expr::expressions::Column as PhysicalColumn,
    physical_plan::{execution_plan::reset_plan_states, ExecutionPlan},
};
use futures::{
    channel::oneshot,
    future::{self, BoxFuture, FutureExt},
};

use crate::{
    cut::{cut_requests, shapes, substitute, with_results, Kept, Shape},
    graph::{GraphInner, QueryGraph, QueryNode},
    key::{address, resolution_code, KeyParts, NodeKey, SelectionKey},
    reads::reads_owner,
    sources,
    store::{lock, Abort, Computed, Flight, FlightResult, ResultStore},
};

/// The inputs of a run: the values of the parameters, and the states of the selections by the
/// names of their placeholders without `$`.
#[derive(Clone, Debug)]
pub struct Inputs {
    pub params: ParamValues,
    pub selections: BTreeMap<String, SelectionState>,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            params: ParamValues::List(Vec::new()),
            selections: BTreeMap::new(),
        }
    }
}

/// The plans of a graph's kept nodes. A run plans a node the first time that it computes the
/// node for a [`MemoKey`], and later runs of the graph reuse that plan.
#[derive(Default)]
pub(crate) struct Memos {
    entries: HashMap<MemoKey, Arc<Memo>>,
}

/// The most plans that a graph holds. A graph that reaches this number drops all its plans and
/// plans its nodes again as its runs compute them.
const MEMOS: usize = 1024;

/// What a node's plans depend on besides the node's subplan: the columns that a run computes, the
/// values of the parameters that the subplan reads, and the versions of the sources that it
/// scans. The states of the selections are not part of the key, because a computation binds them
/// to the plan.
#[derive(Clone, PartialEq, Eq, Hash)]
struct MemoKey {
    node: usize,
    columns: Vec<usize>,
    params: Vec<ScalarValue>,
    versions: Vec<u64>,
}

/// The physical plans of a node's memo, by the shapes of its inputs' results.
type Physical = HashMap<Vec<Option<Shape>>, Arc<dyn ExecutionPlan>>;

/// The plans of a node for one [`MemoKey`]. `logical` is the node's optimized plan, with its
/// `selected` calls replaced by selection nodes. `physical` holds a physical plan for each
/// combination of shapes of the node's inputs that a computation met, with its selections'
/// operators unbound. Each computation binds a copy of the physical plan to the states of its run.
pub(crate) struct Memo {
    logical: LogicalPlan,
    physical: std::sync::Mutex<Physical>,
}

/// A run's batches of each plan, the nodes that it resolved, and what it did with each.
#[derive(Clone, Debug)]
pub struct Run {
    /// The batches of each plan, in the order of the plans that `compile` took.
    pub batches: Vec<Vec<RecordBatch>>,
    /// The nodes that the run resolved.
    pub nodes: Arc<[QueryNode]>,
    /// The outcome of each node, in the order of `nodes`.
    pub outcomes: Vec<Outcome>,
}

impl Run {
    /// A Graphviz DOT graph of the run's nodes, with the outcome of each.
    pub fn to_dot(&self) -> String {
        crate::dot::to_dot(&self.nodes, Some(&self.outcomes))
    }
}

/// What a run did with a node.
#[derive(Clone, Debug)]
pub enum Outcome {
    /// A kept result served the node.
    Hit(Output),
    /// The node was computed, by the run or by a computation that it waited for, and its result
    /// is kept. `widened` holds the columns that a kept result lacked, when the node computed
    /// because of them, and is empty otherwise.
    Computed {
        output: Output,
        elapsed: Duration,
        widened: Vec<String>,
    },
    /// The node was computed, and its result is not kept: the node calls a volatile or a stable
    /// function, its result is larger than the store keeps, or a table that it scans changed
    /// while it computed.
    Uncached { output: Output, elapsed: Duration },
    /// The node is below a hit, and the run did not resolve it.
    NotRead,
}

/// A node's result as a run used it.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub rows: usize,
    pub bytes: usize,
    pub columns: Vec<String>,
}

/// The key of a run: the keys of its plans' top nodes, which identify the inputs that its plans
/// read. Runs with equal keys return the same rows. What they compute depends on the results that
/// the store keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct RunKey {
    tops: Vec<TopKey>,
}

#[derive(Clone, Debug, PartialEq)]
enum TopKey {
    Kept(NodeKey),
    /// A top node that is not kept, whose runs can return other rows.
    Fresh(u64),
}

static FRESH: AtomicU64 = AtomicU64::new(0);

/// The versions of the store's sources, by the addresses of their providers.
fn versions(graph: &GraphInner) -> HashMap<usize, u64> {
    let state = graph.store.state();
    (state.sources.iter())
        .map(|(address, source)| (*address, source.version))
        .collect()
}

/// The values of the parameters that `node` reads, in the order of its reads.
fn param_values(node: &QueryNode, inputs: &Inputs) -> Result<Vec<ScalarValue>> {
    (node.reads.params.iter())
        .map(|id| Ok(inputs.params.get_placeholders_with_values(id)?.value))
        .collect()
}

/// The versions of the sources that `node` scans, in the order of its sources.
fn source_versions(node: &QueryNode, versions: &HashMap<usize, u64>) -> Vec<u64> {
    (node.sources.iter())
        .map(|(_, provider)| versions.get(&address(provider)).copied().unwrap_or(0))
        .collect()
}

/// The key of `node` for `inputs`.
fn node_key(
    graph: &GraphInner,
    node: &QueryNode,
    inputs: &Inputs,
    versions: &HashMap<usize, u64>,
) -> Result<NodeKey> {
    let params = param_values(node, inputs)?;
    let selections = (node.reads.selections.iter())
        .map(|(parameter, owners)| {
            let state = selection_state(inputs, parameter)?;
            let clauses: Vec<u64> = (state.clauses.iter())
                .filter(|clause| reads_owner(owners, &clause.owner))
                .map(|clause| clause.fingerprint())
                .collect();
            // A test that keeps no clause selects every row, whatever the resolution.
            let resolution = match clauses.is_empty() {
                true => None,
                false => Some(resolution_code(state.resolution)),
            };
            Ok(SelectionKey {
                resolution,
                clauses,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let versions = source_versions(node, versions);
    Ok(NodeKey::new(KeyParts {
        identity: node.identity.clone(),
        params,
        selections,
        versions,
        settings: graph.settings,
    }))
}

/// The state of the selection whose placeholder is `parameter`.
fn selection_state<'a>(inputs: &'a Inputs, parameter: &str) -> Result<&'a SelectionState> {
    match inputs.selections.get(parameter.trim_start_matches('$')) {
        Some(state) => Ok(state),
        None => plan_err!("no state for the selection {parameter}"),
    }
}

pub(crate) fn run_key(graph: &QueryGraph, inputs: &Inputs) -> Result<RunKey> {
    let inner = &graph.inner;
    let versions = versions(inner);
    let tops = (inner.tops.iter())
        .map(|top| {
            let node = &inner.nodes[*top];
            Ok(match node.kept {
                true => TopKey::Kept(node_key(inner, node, inputs, &versions)?),
                false => TopKey::Fresh(FRESH.fetch_add(1, Ordering::Relaxed)),
            })
        })
        .collect::<Result<_>>()?;
    Ok(RunKey { tops })
}

/// A wait for a computation, by a run or by a computation that reads its result, which counts as
/// a waiter until it is dropped. A computation without waiters stops at the next run of its
/// graph, unless that run waits for it.
struct Waiting(Arc<Flight>);

impl Waiting {
    fn new(flight: &Arc<Flight>) -> Self {
        *lock(&flight.waiters) += 1;
        Self(flight.clone())
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut waiters = lock(&self.0.waiters);
        *waiters = waiters.saturating_sub(1);
    }
}

type KeptFuture = BoxFuture<'static, Result<Arc<Kept>, Arc<DataFusionError>>>;

/// What a computation needs once the nodes below it are resolved: its plan, the result of each
/// cut that the plan scans, and its waits for the computations of those results, which it holds
/// until it finishes.
struct Job {
    plan: Planned,
    inputs: Vec<Option<KeptFuture>>,
    _waiting: Vec<Waiting>,
}

/// The plan of a computation, whose `selected` calls are selection nodes, and the inputs of its
/// run.
enum Planned {
    /// The memo of a kept node. The computation reuses the memo's physical plan and binds it to
    /// the states of its run.
    Memo {
        memo: Arc<Memo>,
        inputs: Arc<Inputs>,
    },
    /// The optimized plan of a node that is not kept, which calls a volatile or a stable
    /// function. Each computation plans it, in a session that holds the states of its run.
    PerRun {
        plan: LogicalPlan,
        inputs: Arc<Inputs>,
    },
}

impl Planned {
    fn logical(&self) -> &LogicalPlan {
        match self {
            Planned::Memo { memo, .. } => &memo.logical,
            Planned::PerRun { plan, .. } => plan,
        }
    }
}

/// A computation's job before the run sends it: the sender, the optimized plan, and whether the
/// plan scans each of the node's cuts.
struct Unsent {
    sender: oneshot::Sender<Job>,
    plan: Planned,
    present: Vec<bool>,
}

/// How a run resolved a node.
enum Resolution {
    NotRead,
    Hit(Arc<Kept>),
    /// A computation, which the run started, with the job that it still has to send, or which
    /// it waits for.
    Computing {
        flight: Arc<Flight>,
        job: Option<Box<Unsent>>,
    },
}

/// The columns `columns` of `node`'s plan.
fn projected(node: &QueryNode, columns: &BTreeSet<usize>) -> Result<LogicalPlan> {
    let schema = node.plan.schema();
    match columns.len() == schema.fields().len() {
        true => Ok(node.plan.clone()),
        false => {
            let exprs: Vec<Expr> = (columns.iter())
                .map(|index| Expr::Column(Column::from(schema.qualified_field(*index))))
                .collect();
            LogicalPlanBuilder::from(node.plan.clone())
                .project(exprs)?
                .build()
        }
    }
}

/// `plan`, a node's plan, with selection nodes over the fields that `graph` holds, its parameters
/// replaced by their values in `inputs`, and optimized with `state`.
fn optimized(
    graph: &GraphInner,
    plan: LogicalPlan,
    inputs: &Inputs,
    state: &SessionState,
) -> Result<LogicalPlan> {
    let plan =
        with_selection_nodes(plan, &graph.fields)?.with_param_values(inputs.params.clone())?;
    state.optimize(&plan)
}

/// The plan of a computation of the columns `columns` of the node at `index` for `inputs`. For a
/// kept node, the plan is the graph's memo for those columns, the values of the parameters that
/// the node reads, and the versions of its sources, which this function creates on first use.
/// For another node, the plan is the node's optimized plan.
fn planned(
    graph: &GraphInner,
    index: usize,
    node: &QueryNode,
    columns: &BTreeSet<usize>,
    inputs: &Arc<Inputs>,
    versions: &HashMap<usize, u64>,
    state: &SessionState,
) -> Result<Planned> {
    let plan = projected(node, columns)?;
    if !node.kept {
        return Ok(Planned::PerRun {
            plan: optimized(graph, plan, inputs, state)?,
            inputs: inputs.clone(),
        });
    }
    let key = MemoKey {
        node: index,
        columns: columns.iter().copied().collect(),
        params: param_values(node, inputs)?,
        versions: source_versions(node, versions),
    };
    let found = lock(&graph.memos).entries.get(&key).cloned();
    let memo = match found {
        Some(memo) => memo,
        None => {
            let memo = Arc::new(Memo {
                logical: optimized(graph, plan, inputs, state)?,
                physical: std::sync::Mutex::new(HashMap::new()),
            });
            let mut memos = lock(&graph.memos);
            if memos.entries.len() >= MEMOS {
                memos.entries.clear();
            }
            memos.entries.insert(key, memo.clone());
            memo
        }
    };
    Ok(Planned::Memo {
        memo,
        inputs: inputs.clone(),
    })
}

/// A copy of `state` that plans without dynamic filters, for the physical plans that a graph's
/// computations reuse. DataFusion does not support a second execution of a plan that uses
/// dynamic filters, also after `reset_plan_states`.
fn reusable(state: &SessionState) -> SessionState {
    let mut state = state.clone();
    let optimizer = &mut state.config_mut().options_mut().optimizer;
    optimizer.enable_dynamic_filter_pushdown = false;
    optimizer.enable_topk_dynamic_filter_pushdown = false;
    optimizer.enable_join_dynamic_filter_pushdown = false;
    optimizer.enable_aggregate_dynamic_filter_pushdown = false;
    state
}

/// The physical plan of a computation whose inputs' results are `results`, with its selections'
/// operators bound to the states of its run.
///
/// A node that is not kept is planned here, in a session that binds the operators as it plans. A
/// kept node reuses the physical plan that its memo holds for the shapes of `results`, with the
/// leaf of each cut reading its result in `results`, and plans one when the memo holds none.
async fn physical(
    plan: Planned,
    results: &[Option<Arc<Kept>>],
    state: &SessionState,
) -> Result<Arc<dyn ExecutionPlan>> {
    let (memo, inputs) = match plan {
        Planned::PerRun { plan, inputs } => {
            let plan = with_results(plan, results)?;
            let session = with_selection_states(state, inputs.selections.clone());
            return (session.query_planner())
                .create_physical_plan(&plan, &session)
                .await;
        }
        Planned::Memo { memo, inputs } => (memo, inputs),
    };
    let target = state.config_options().execution.target_partitions;
    let shape = shapes(results, target);
    let found = lock(&memo.physical).get(&shape).cloned();
    let plan = match found {
        Some(plan) => substitute(plan, results, target)?,
        None => {
            let plan = with_results(memo.logical.clone(), results)?;
            let session = reusable(state);
            let plan = (session.query_planner())
                .create_physical_plan(&plan, &session)
                .await?;
            lock(&memo.physical).insert(shape, plan.clone());
            plan
        }
    };
    reset_plan_states(bind_selections(plan, &inputs.selections)?)
}

/// Removes a computation from the store when it is dropped: when its task finishes, panics, or
/// stops. A removal that the store made already does nothing.
struct Landing {
    store: ResultStore,
    key: Option<NodeKey>,
    id: u64,
}

impl Drop for Landing {
    fn drop(&mut self) {
        self.store.state().land(&self.key, self.id);
    }
}

type Task = tokio::task::JoinHandle<Result<Arc<Computed>>>;

/// The parts of a computation that exist before its task: the result, which waits to be handed
/// the task, and the sender of that handoff.
fn result() -> (FlightResult, oneshot::Sender<Task>) {
    let (sender, receiver) = oneshot::channel::<Task>();
    let result = async move {
        let task = receiver.await.map_err(|_| {
            Arc::new(DataFusionError::Execution(
                "the computation's task was never spawned".into(),
            ))
        })?;
        match task.await {
            Ok(result) => result.map_err(Arc::new),
            Err(error) => Err(Arc::new(DataFusionError::External(Box::new(error)))),
        }
    }
    .boxed()
    .shared();
    (result, sender)
}

/// Spawn the task of the computation `flight` of a node: it waits for its job, then computes the
/// node, keeps its result under `key` when the node is kept, and leaves the store. The caller
/// does not hold the store's lock, because Tokio drops the task at once when the runtime is
/// shutting down, and its guard then takes the lock to remove the computation.
#[allow(clippy::too_many_arguments)]
fn spawn(
    store: &ResultStore,
    state: &SessionState,
    flight: &Flight,
    key: Option<NodeKey>,
    handoff: oneshot::Sender<Task>,
    receiver: oneshot::Receiver<Job>,
    columns: Vec<usize>,
    widened: Vec<String>,
) {
    let store = store.clone();
    let state = state.clone();
    // The computation leaves the store when its task finishes, fails, panics, or stops, even
    // before the task's first poll, since the task owns the guard from its start.
    let landing = Landing {
        store: store.clone(),
        key: key.clone(),
        id: flight.id,
    };
    let task = tokio::spawn(async move {
        let landing = landing;
        let computed = compute(&store, &state, receiver, key.as_ref(), columns, widened).await;
        drop(landing);
        computed
    });
    flight.spawned(task.abort_handle());
    let _ = handoff.send(task);
}

/// Wait for a computation's job, then compute the node and keep its result under `key`.
async fn compute(
    store: &ResultStore,
    state: &SessionState,
    receiver: oneshot::Receiver<Job>,
    key: Option<&NodeKey>,
    columns: Vec<usize>,
    widened: Vec<String>,
) -> Result<Arc<Computed>> {
    let job = receiver.await.map_err(|_| {
        DataFusionError::Execution("the run that started the computation ended".into())
    })?;
    let mut results = Vec::with_capacity(job.inputs.len());
    let mut pending = Vec::new();
    for input in job.inputs {
        match input {
            Some(input) => {
                pending.push(input);
                results.push(true);
            }
            None => results.push(false),
        }
    }
    let mut computed = (future::try_join_all(pending).await)
        .map_err(DataFusionError::Shared)?
        .into_iter();
    let results: Vec<Option<Arc<Kept>>> = (results.into_iter())
        .map(|present| present.then(|| computed.next().expect("a result for each cut")))
        .collect();
    let started = Instant::now();
    let physical = physical(job.plan, &results, state).await?;
    // One partition gives the rows in an order that a reader can depend on, and the plan can
    // declare which order it is.
    let properties = physical.properties();
    let ordered = properties.output_partitioning().partition_count() == 1;
    let ordering = match ordered {
        true => (properties.output_ordering().into_iter().flatten())
            .map_while(|sort| {
                let column = sort.expr.downcast_ref::<PhysicalColumn>()?;
                Some((columns[column.index()], sort.options))
            })
            .collect(),
        false => Vec::new(),
    };
    let batches = datafusion::physical_plan::collect(physical, state.task_ctx()).await?;
    let kept = Arc::new(Kept::new(columns, batches, ordered, ordering));
    let elapsed = started.elapsed();
    let stored = match key {
        Some(key) => (store.state()).keep(store.config(), key.clone(), kept.clone()),
        None => false,
    };
    Ok(Arc::new(Computed {
        kept,
        elapsed,
        widened,
        stored,
    }))
}

/// The kept result of a resolved node, as the input of a job.
fn input_of(resolution: &Resolution) -> KeptFuture {
    match resolution {
        Resolution::Hit(kept) => future::ready(Ok(kept.clone())).boxed(),
        Resolution::Computing { flight, .. } => {
            let result = flight.result.clone();
            async move { result.await.map(|computed| computed.kept.clone()) }.boxed()
        }
        Resolution::NotRead => future::ready(Err(Arc::new(DataFusionError::Internal(
            "a job reads a node that the run did not resolve".into(),
        ))))
        .boxed(),
    }
}

/// The names of the columns `columns` of `node`'s output.
fn names(node: &QueryNode, columns: impl IntoIterator<Item = usize>) -> Vec<String> {
    let schema = node.plan.schema();
    (columns.into_iter())
        .map(|index| schema.field(index).name().clone())
        .collect()
}

fn output(node: &QueryNode, kept: &Kept) -> Output {
    Output {
        rows: kept.rows,
        bytes: kept.bytes,
        columns: names(node, kept.columns.iter().copied()),
    }
}

/// Adds the columns that the optimized `plan` of `node` reads through each of its cuts to the
/// requests of the nodes below the cuts, and marks those nodes as needed. Returns whether the
/// plan scans each cut, by position: a cut that the optimizer removed requests nothing, and its
/// node is not read.
fn request(
    node: &QueryNode,
    plan: &LogicalPlan,
    requests: &mut [BTreeSet<usize>],
    needed: &mut [bool],
) -> Result<Vec<bool>> {
    let wanted = cut_requests(plan, node.inputs.len())?;
    for (position, columns) in wanted.iter().enumerate() {
        let input = node.inputs[position];
        if let Some(columns) = columns {
            requests[input].extend(columns.iter().copied());
            needed[input] = true;
        }
    }
    Ok(wanted.iter().map(Option::is_some).collect())
}

pub(crate) async fn run(graph: &QueryGraph, inputs: &Inputs) -> Result<Run> {
    let inner = &graph.inner;
    let read: BTreeSet<usize> = (inner.nodes.iter())
        .flat_map(|node| node.sources.iter().map(|(_, provider)| address(provider)))
        .collect();
    sources::check(&inner.store, &read.into_iter().collect::<Vec<_>>()).await?;

    let nodes = inner.nodes.clone();
    let shared = Arc::new(inputs.clone());
    let mut state = inner.state.clone();
    state.mark_start_execution();

    let versions = versions(inner);
    let keys = (nodes.iter())
        .map(|node| match node.kept {
            true => node_key(inner, node, inputs, &versions).map(Some),
            false => Ok(None),
        })
        .collect::<Result<Vec<_>>>()?;

    // The first pass: resolve the nodes from the top down, so each node's readers have requested
    // their columns before it resolves.
    let count = nodes.len();
    let mut requests: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
    let mut needed = vec![false; count];
    for top in &inner.tops {
        needed[*top] = true;
        requests[*top] = (0..nodes[*top].plan.schema().fields().len()).collect();
    }
    let mut resolutions: Vec<Resolution> = (0..count).map(|_| Resolution::NotRead).collect();
    let mut waiting = Vec::new();
    let mut started: Vec<(Option<NodeKey>, Arc<Flight>)> = Vec::new();
    let failure = (|| -> Result<()> {
        for index in (0..count).rev() {
            if !needed[index] {
                continue;
            }
            let node = &nodes[index];
            let mut columns = requests[index].clone();
            if columns.is_empty() && !node.plan.schema().fields().is_empty() {
                // A reader that needs only the rows, such as `count(*)`, keeps one column. A
                // node without columns keeps only its rows.
                columns.insert(0);
            }
            let mut store = inner.store.state();
            if let Some(key) = &keys[index] {
                let found = match store.hit(key, &columns) {
                    Some(kept) => Some(Resolution::Hit(kept)),
                    None => store.flight(key, &columns).map(|flight| {
                        waiting.push(Waiting::new(&flight));
                        Resolution::Computing { flight, job: None }
                    }),
                };
                if let Some(found) = found {
                    drop(store);
                    resolutions[index] = found;
                    continue;
                }
            }
            // A miss: compute the requested columns, and the columns that the result or the
            // computations of the key hold, so the new result replaces them.
            let mut widened = Vec::new();
            if let Some(key) = &keys[index] {
                if let Some(kept) = store.kept_columns(key) {
                    widened = names(node, columns.iter().copied().filter(|c| !kept.contains(c)));
                    columns.extend(kept);
                }
                columns.extend(store.flight_columns(key));
                // A narrower computation that no run waits for stops.
                let bucket = Some(key.clone());
                let narrower: Vec<Arc<Flight>> = (store.flights.get(&bucket).into_iter())
                    .flatten()
                    .filter(|flight| *lock(&flight.waiters) == 0)
                    .cloned()
                    .collect();
                for flight in narrower {
                    flight.stop();
                    store.land(&bucket, flight.id);
                }
            }
            store.next_flight += 1;
            let id = store.next_flight;
            let key = keys[index].clone();
            let (result, handoff) = result();
            // Every computation is in the store, so the next run can stop it, and only a kept
            // node's computation is under its key, which other runs can join. It is registered
            // before its task is spawned, and the task is spawned after the lock is released.
            let flight = Arc::new(Flight {
                id,
                columns: columns.clone(),
                result,
                abort: std::sync::Mutex::new(Abort::default()),
                waiters: std::sync::Mutex::new(0),
                reads: std::sync::Mutex::new(Vec::new()),
                graph: inner.id,
            });
            waiting.push(Waiting::new(&flight));
            (store.flights.entry(key.clone()).or_default()).push(flight.clone());
            started.push((key.clone(), flight.clone()));
            drop(store);
            let (sender, receiver) = oneshot::channel::<Job>();
            spawn(
                &inner.store,
                &state,
                &flight,
                key,
                handoff,
                receiver,
                columns.iter().copied().collect(),
                widened,
            );
            let plan = planned(inner, index, node, &columns, &shared, &versions, &state)?;
            let present = request(node, plan.logical(), &mut requests, &mut needed)?;
            resolutions[index] = Resolution::Computing {
                flight,
                job: Some(Box::new(Unsent {
                    sender,
                    plan,
                    present,
                })),
            };
        }
        Ok(())
    })();
    if let Err(error) = failure {
        // The computations that this run started stop and leave the store, so no other run
        // waits for them.
        let mut store = inner.store.state();
        for (key, flight) in &started {
            flight.stop();
            store.land(key, flight.id);
        }
        return Err(error);
    }

    // The second pass: give each computation the results of the cuts that its plan scans, and
    // its waits for the computations of those results.
    for index in 0..count {
        let Resolution::Computing { job, .. } = &mut resolutions[index] else {
            continue;
        };
        let Some(unsent) = job.take() else {
            continue;
        };
        let Unsent {
            sender,
            plan,
            present,
        } = *unsent;
        let node = &nodes[index];
        let mut job_inputs = Vec::new();
        let mut job_waiting = Vec::new();
        for (input, present) in node.inputs.iter().zip(present) {
            let resolution = &resolutions[*input];
            if !present {
                job_inputs.push(None);
                continue;
            }
            if let Resolution::Computing { flight, .. } = resolution {
                job_waiting.push(Waiting::new(flight));
            }
            job_inputs.push(Some(input_of(resolution)));
        }
        if let Resolution::Computing { flight, .. } = &resolutions[index] {
            *lock(&flight.reads) = job_waiting
                .iter()
                .map(|waiting| waiting.0.clone())
                .collect();
        }
        let _ = sender.send(Job {
            plan,
            inputs: job_inputs,
            _waiting: job_waiting,
        });
    }

    // The run has computed its keys, and its computations wait for the computations that they
    // read: the graph's computations that nothing waits for stop, with the computations that
    // only they wait for.
    inner.store.state().stop_idle(Some(inner.id));

    let mut outcomes = Vec::with_capacity(count);
    let mut results: Vec<Option<Arc<Kept>>> = vec![None; count];
    let mut failed = None;
    for (index, resolution) in resolutions.iter().enumerate() {
        let node = &nodes[index];
        let outcome = match resolution {
            Resolution::NotRead => Outcome::NotRead,
            Resolution::Hit(kept) => {
                results[index] = Some(kept.clone());
                Outcome::Hit(output(node, kept))
            }
            Resolution::Computing { flight, .. } => match flight.result.clone().await {
                Ok(computed) => {
                    results[index] = Some(computed.kept.clone());
                    let output = output(node, &computed.kept);
                    match computed.stored {
                        true => Outcome::Computed {
                            output,
                            elapsed: computed.elapsed,
                            widened: computed.widened.clone(),
                        },
                        false => Outcome::Uncached {
                            output,
                            elapsed: computed.elapsed,
                        },
                    }
                }
                Err(error) => {
                    failed.get_or_insert(error);
                    Outcome::NotRead
                }
            },
        };
        outcomes.push(outcome);
    }
    drop(waiting);
    if let Some(error) = failed {
        return Err(DataFusionError::Shared(error));
    }

    let batches = (inner.tops.iter())
        .map(|top| {
            let node = &nodes[*top];
            let kept = results[*top].as_ref().expect("a top node resolves");
            let schema: SchemaRef = Arc::new(node.plan.schema().as_arrow().clone());
            let columns: Vec<usize> = (0..schema.fields().len()).collect();
            kept.project(&columns, schema)
        })
        .collect::<Result<_>>()?;
    Ok(Run {
        batches,
        nodes: nodes.clone(),
        outcomes,
    })
}

#[cfg(test)]
mod tests;
