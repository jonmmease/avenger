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
use avenger_selection_datafusion::{bind_selections, preaggregate::Focus};
use datafusion::{
    arrow::{array::RecordBatch, datatypes::SchemaRef},
    common::{plan_err, Column, DataFusionError, ParamValues, Result},
    execution::SessionState,
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
    physical_expr::expressions::Column as PhysicalColumn,
};
use futures::{
    channel::oneshot,
    future::{self, BoxFuture, FutureExt},
};

use crate::{
    cut::{cut_requests, with_results, Kept},
    graph::{GraphInner, Nodes, QueryGraph, QueryNode},
    key::{address, resolution_code, KeyParts, NodeKey, SelectionKey},
    reads::reads_owner,
    sources,
    store::{lock, Computed, Flight, FlightResult, ResultStore},
};

/// The inputs of a run: the values of the parameters, the states of the selections by the names
/// of their placeholders without `$`, and the focus.
#[derive(Clone, Debug)]
pub struct Inputs {
    pub params: ParamValues,
    pub selections: BTreeMap<String, SelectionState>,
    /// The selection that is about to move. It decides how the run computes its results and never
    /// what they hold.
    pub focus: Option<Focus>,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            params: ParamValues::List(Vec::new()),
            selections: BTreeMap::new(),
            focus: None,
        }
    }
}

/// A run's batches of each plan, the nodes that it resolved, and what it did with each.
#[derive(Clone, Debug)]
pub struct Run {
    /// The batches of each plan, in the order of the plans that `compile` took.
    pub batches: Vec<Vec<RecordBatch>>,
    /// The nodes that the run resolved: the direct plans', or the variant's for the focus.
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
    /// function, or its result is larger than the store keeps.
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

/// The key of a run: the keys of its plans' top nodes and its focus, which identify the inputs
/// that its plans read. Runs with equal keys return the same rows. What they compute depends on
/// the results that the store keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct RunKey {
    tops: Vec<TopKey>,
    focus: Option<Focus>,
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

/// The key of `node` for `inputs`.
fn node_key(
    graph: &GraphInner,
    node: &QueryNode,
    inputs: &Inputs,
    versions: &HashMap<usize, u64>,
) -> Result<NodeKey> {
    let params = (node.reads.params.iter())
        .map(|id| Ok(inputs.params.get_placeholders_with_values(id)?.value))
        .collect::<Result<Vec<_>>>()?;
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
    let versions = (node.sources.iter())
        .map(|(_, provider)| versions.get(&address(provider)).copied().unwrap_or(0))
        .collect();
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
    let tops = (inner.direct.tops.iter())
        .map(|top| {
            let node = &inner.direct.nodes[*top];
            Ok(match node.kept {
                true => TopKey::Kept(node_key(inner, node, inputs, &versions)?),
                false => TopKey::Fresh(FRESH.fetch_add(1, Ordering::Relaxed)),
            })
        })
        .collect::<Result<_>>()?;
    Ok(RunKey {
        tops,
        focus: inputs.focus.clone(),
    })
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
    plan: LogicalPlan,
    inputs: Vec<Option<KeptFuture>>,
    _waiting: Vec<Waiting>,
}

/// A computation's job before the run sends it: the sender, the optimized plan, and whether the
/// plan scans each of the node's cuts.
struct Unsent {
    sender: oneshot::Sender<Job>,
    plan: LogicalPlan,
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

/// The columns `columns` of `node`'s plan, bound for `inputs` and optimized with `state`.
fn planned(
    node: &QueryNode,
    columns: &BTreeSet<usize>,
    inputs: &Inputs,
    state: &SessionState,
) -> Result<LogicalPlan> {
    let schema = node.plan.schema();
    let plan = match columns.len() == schema.fields().len() {
        true => node.plan.clone(),
        false => {
            let exprs: Vec<Expr> = (columns.iter())
                .map(|index| Expr::Column(Column::from(schema.qualified_field(*index))))
                .collect();
            LogicalPlanBuilder::from(node.plan.clone())
                .project(exprs)?
                .build()?
        }
    };
    let states: Vec<(&str, &SelectionState)> = (inputs.selections.iter())
        .map(|(name, state)| (name.as_str(), state))
        .collect();
    let plan = bind_selections(plan, &states)?.with_param_values(inputs.params.clone())?;
    state.optimize(&plan)
}

/// Start the computation `id` of a node: a task that waits for its job, then computes the node,
/// keeps its result under `key` when the node is kept, and leaves the store's computations.
fn start(
    store: &ResultStore,
    state: &SessionState,
    key: Option<NodeKey>,
    id: u64,
    columns: Vec<usize>,
    widened: Vec<String>,
) -> (FlightResult, oneshot::Sender<Job>, tokio::task::AbortHandle) {
    let (sender, receiver) = oneshot::channel::<Job>();
    let store = store.clone();
    let state = state.clone();
    let task = tokio::spawn(async move {
        let computed = compute(&store, &state, receiver, key.as_ref(), columns, widened).await;
        store.state().land(&key, id);
        computed
    });
    let abort = task.abort_handle();
    let result = async move {
        match task.await {
            Ok(result) => result.map_err(Arc::new),
            Err(error) => Err(Arc::new(DataFusionError::External(Box::new(error)))),
        }
    }
    .boxed()
    .shared();
    (result, sender, abort)
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
    let plan = with_results(job.plan, &results)?;
    let physical = state
        .query_planner()
        .create_physical_plan(&plan, state)
        .await?;
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

/// Add the requests of the optimized `plan` of `node` to the nodes that it reads, when `only`
/// accepts them. A cut that the optimizer removed requests nothing, and its node is not read.
/// The presence of each cut in the plan, by position.
fn request(
    nodes: &Nodes,
    node: &QueryNode,
    plan: &LogicalPlan,
    requests: &mut [BTreeSet<usize>],
    needed: &mut [bool],
    only: impl Fn(&QueryNode) -> bool,
) -> Result<Vec<bool>> {
    let wanted = cut_requests(plan, node.inputs.len())?;
    for (position, columns) in wanted.iter().enumerate() {
        let input = node.inputs[position];
        if let Some(columns) = columns {
            if only(&nodes.nodes[input]) {
                requests[input].extend(columns.iter().copied());
                needed[input] = true;
            }
        }
    }
    Ok(wanted.iter().map(Option::is_some).collect())
}

/// Stop the computations of `graph` that no run waits for, and each computation of the graph
/// whose waiters are only computations that stop.
fn stop_idle(graph: &GraphInner) {
    let mut store = graph.store.state();
    let all: Vec<(Option<NodeKey>, Arc<Flight>)> = (store.flights.iter())
        .flat_map(|(key, flights)| {
            (flights.iter()).map(move |flight| (key.clone(), flight.clone()))
        })
        .collect();
    let mut waiters: HashMap<u64, usize> = (all.iter())
        .map(|(_, flight)| (flight.id, *lock(&flight.waiters)))
        .collect();
    let mut stopping: Vec<Arc<Flight>> = (all.iter())
        .filter(|(_, flight)| flight.graph == graph.id && waiters[&flight.id] == 0)
        .map(|(_, flight)| flight.clone())
        .collect();
    let mut stopped: BTreeSet<u64> = stopping.iter().map(|flight| flight.id).collect();
    while let Some(flight) = stopping.pop() {
        for read in lock(&flight.reads).iter() {
            let Some(count) = waiters.get_mut(&read.id) else {
                continue;
            };
            *count = count.saturating_sub(1);
            if *count == 0 && read.graph == graph.id && stopped.insert(read.id) {
                stopping.push(read.clone());
            }
        }
    }
    for (key, flight) in all {
        if stopped.contains(&flight.id) {
            flight.abort.abort();
            store.land(&key, flight.id);
        }
    }
}

pub(crate) async fn run(graph: &QueryGraph, inputs: &Inputs) -> Result<Run> {
    let inner = &graph.inner;
    let read: BTreeSet<usize> = (inner.direct.nodes.iter())
        .flat_map(|node| node.sources.iter().map(|(_, provider)| address(provider)))
        .collect();
    sources::check(&inner.store, &read.into_iter().collect::<Vec<_>>()).await?;

    let nodes: Arc<Nodes> = match &inputs.focus {
        Some(focus) if focus.serves(selection_state(inputs, &focus.parameter)?) => {
            graph.resolved(Some(focus))?
        }
        _ => inner.direct.clone(),
    };
    let forced = !Arc::ptr_eq(&nodes, &inner.direct);
    let mut state = inner.state.clone();
    state.mark_start_execution();

    let versions = versions(inner);
    let keys = (nodes.nodes.iter())
        .map(|node| match node.kept {
            true => node_key(inner, node, inputs, &versions).map(Some),
            false => Ok(None),
        })
        .collect::<Result<Vec<_>>>()?;

    // The first pass: resolve the nodes from the top down, so each node's readers have requested
    // their columns before it resolves.
    let count = nodes.nodes.len();
    let mut requests: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
    let mut needed = vec![false; count];
    for top in &nodes.tops {
        needed[*top] = true;
        requests[*top] = (0..nodes.nodes[*top].plan.schema().fields().len()).collect();
    }
    if forced {
        // The cells resolve for the first move, through the nodes of their rollups with every
        // column, which a move can read even when this run reads none of them.
        for (index, node) in nodes.nodes.iter().enumerate() {
            if !node.replaces.is_empty() {
                needed[index] = true;
                requests[index] = (0..node.plan.schema().fields().len()).collect();
            }
        }
    }
    let mut resolutions: Vec<Resolution> = (0..count).map(|_| Resolution::NotRead).collect();
    let mut waiting = Vec::new();
    let mut started: Vec<(Option<NodeKey>, Arc<Flight>)> = Vec::new();
    let failure = (|| -> Result<()> {
        for index in (0..count).rev() {
            if !needed[index] {
                continue;
            }
            let node = &nodes.nodes[index];
            let mut columns = requests[index].clone();
            if columns.is_empty() {
                // A reader that needs only the rows, such as `count(*)`, keeps one column.
                columns.insert(0);
            }
            let reads_cells = forced && node.inputs.iter().any(|input| nodes.nodes[*input].cells);
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
                    if reads_cells {
                        // The cells below the hit resolve for the first move.
                        let plan = planned(node, &columns, inputs, &state)?;
                        request(&nodes, node, &plan, &mut requests, &mut needed, |n| n.cells)?;
                    }
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
                    flight.abort.abort();
                    store.land(&bucket, flight.id);
                }
            }
            store.next_flight += 1;
            let id = store.next_flight;
            let key = keys[index].clone();
            let (result, sender, abort) = start(
                &inner.store,
                &state,
                key.clone(),
                id,
                columns.iter().copied().collect(),
                widened,
            );
            // Every computation is in the store, so the next run can stop it, and only a kept
            // node's computation is under its key, which other runs can join.
            let flight = Arc::new(Flight {
                id,
                columns: columns.clone(),
                result,
                abort,
                waiters: std::sync::Mutex::new(0),
                reads: std::sync::Mutex::new(Vec::new()),
                graph: inner.id,
            });
            waiting.push(Waiting::new(&flight));
            (store.flights.entry(key.clone()).or_default()).push(flight.clone());
            started.push((key, flight.clone()));
            drop(store);
            let plan = planned(node, &columns, inputs, &state)?;
            let present = request(&nodes, node, &plan, &mut requests, &mut needed, |_| true)?;
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
            flight.abort.abort();
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
        let node = &nodes.nodes[index];
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
    stop_idle(inner);

    let mut outcomes = Vec::with_capacity(count);
    let mut results: Vec<Option<Arc<Kept>>> = vec![None; count];
    let mut failed = None;
    for (index, resolution) in resolutions.iter().enumerate() {
        let node = &nodes.nodes[index];
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

    let batches = (nodes.tops.iter())
        .map(|top| {
            let node = &nodes.nodes[*top];
            let kept = results[*top].as_ref().expect("a top node resolves");
            let schema: SchemaRef = Arc::new(node.plan.schema().as_arrow().clone());
            let columns: Vec<usize> = (0..schema.fields().len()).collect();
            kept.project(&columns, schema)
        })
        .collect::<Result<_>>()?;
    Ok(Run {
        batches,
        nodes: nodes.nodes.clone(),
        outcomes,
    })
}
