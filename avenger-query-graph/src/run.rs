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
};
use futures::{
    channel::oneshot,
    future::{self, BoxFuture, FutureExt},
};

use crate::{
    cut::{cut_requests, with_results, Kept},
    graph::{GraphInner, Nodes, QueryGraph, QueryNode},
    key::{resolution_code, KeyParts, NodeKey, SelectionKey, TableVersion},
    reads::reads_owner,
    sources,
    store::{lock, Computed, Flight, FlightResult},
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

/// The key of a run: the keys of its plans' top nodes and its focus. Runs with equal keys return
/// the same rows and do the same work.
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

/// The versions of the store's sources.
fn versions(graph: &GraphInner) -> HashMap<String, u64> {
    let state = graph.store.state();
    (state.sources.iter())
        .map(|(name, source)| (name.clone(), source.version))
        .collect()
}

/// The key of `node` for `inputs`.
fn node_key(
    graph: &GraphInner,
    node: &QueryNode,
    inputs: &Inputs,
    versions: &HashMap<String, u64>,
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
    let tables = (node.sources.iter())
        .map(|(name, provider)| TableVersion {
            provider: provider.clone(),
            version: versions.get(name).copied().unwrap_or(0),
        })
        .collect();
    Ok(NodeKey::new(KeyParts {
        identity: node.identity.clone(),
        params,
        selections,
        tables,
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

/// A run's wait for a computation, which counts as a waiter until it is dropped.
struct Waiting(Arc<Flight>);

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut waiters = lock(&self.0.waiters);
        *waiters = waiters.saturating_sub(1);
    }
}

/// What a computation needs once the nodes below it are resolved.
struct Job {
    plan: LogicalPlan,
    inputs: Vec<BoxFuture<'static, Result<Arc<Kept>, Arc<DataFusionError>>>>,
}

/// How a run resolved a node.
enum Resolution {
    NotRead,
    Hit(Arc<Kept>),
    /// A computation, which the run started or waits for.
    Computing {
        result: FlightResult,
        job: Option<(oneshot::Sender<Job>, LogicalPlan)>,
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

/// Start the computation of a node: a task that waits for its job, then computes the node and
/// keeps its result under `key`.
fn start(
    graph: &GraphInner,
    state: &SessionState,
    key: Option<(NodeKey, u64)>,
    columns: Vec<usize>,
    widened: Vec<String>,
) -> (FlightResult, oneshot::Sender<Job>, tokio::task::AbortHandle) {
    let (sender, receiver) = oneshot::channel::<Job>();
    let store = graph.store.clone();
    let state = state.clone();
    let task = tokio::spawn(async move {
        let computed = compute(
            &store,
            &state,
            receiver,
            key.as_ref().map(|(key, _)| key),
            columns,
            widened,
        )
        .await;
        if let Some((key, id)) = &key {
            store.state().land(key, *id);
        }
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
    store: &crate::store::ResultStore,
    state: &SessionState,
    receiver: oneshot::Receiver<Job>,
    key: Option<&NodeKey>,
    columns: Vec<usize>,
    widened: Vec<String>,
) -> Result<Arc<Computed>> {
    {
        let job = receiver.await.map_err(|_| {
            DataFusionError::Execution("the run that started the computation ended".into())
        })?;
        let results = future::try_join_all(job.inputs)
            .await
            .map_err(DataFusionError::Shared)?;
        let started = Instant::now();
        let plan = with_results(job.plan, &results)?;
        let physical = state
            .query_planner()
            .create_physical_plan(&plan, state)
            .await?;
        let batches = datafusion::physical_plan::collect(physical, state.task_ctx()).await?;
        let kept = Arc::new(Kept::new(columns, batches));
        let elapsed = started.elapsed();
        let stored = match key {
            Some(key) => store
                .state()
                .keep(store.config(), key.clone(), kept.clone()),
            None => false,
        };
        Ok(Arc::new(Computed {
            kept,
            elapsed,
            widened,
            stored,
        }))
    }
}

/// The kept result of a resolved node, as the input of a job.
fn input_of(
    resolution: &Resolution,
) -> BoxFuture<'static, Result<Arc<Kept>, Arc<DataFusionError>>> {
    match resolution {
        Resolution::Hit(kept) => future::ready(Ok(kept.clone())).boxed(),
        Resolution::Computing { result, .. } => {
            let result = result.clone();
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

pub(crate) async fn run(graph: &QueryGraph, inputs: &Inputs) -> Result<Run> {
    let inner = &graph.inner;
    let read: BTreeSet<String> = (inner.direct.nodes.iter())
        .flat_map(|node| node.sources.iter().map(|(name, _)| name.clone()))
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
            needed[index] |= node.cells;
            if !node.replaces.is_empty() {
                needed[index] = true;
                requests[index] = (0..node.plan.schema().fields().len()).collect();
            }
        }
    }
    let mut resolutions: Vec<Resolution> = (0..count).map(|_| Resolution::NotRead).collect();
    let mut waiting = Vec::new();
    let mut aborts = Vec::new();
    let failure = (|| -> Result<()> {
        for index in (0..count).rev() {
            if !needed[index] {
                continue;
            }
            let node = &nodes.nodes[index];
            let mut columns = requests[index].clone();
            if columns.is_empty() {
                columns.insert(0);
            }
            let reads_cells = forced && node.inputs.iter().any(|input| nodes.nodes[*input].cells);
            let mut store = inner.store.state();
            if let Some(key) = &keys[index] {
                let found = match store.hit(key, &columns) {
                    Some(kept) => Some(Resolution::Hit(kept)),
                    None => store.flight(key, &columns).map(|flight| {
                        *lock(&flight.waiters) += 1;
                        waiting.push(Waiting(flight.clone()));
                        Resolution::Computing {
                            result: flight.result.clone(),
                            job: None,
                        }
                    }),
                };
                if let Some(found) = found {
                    drop(store);
                    resolutions[index] = found;
                    if reads_cells {
                        // The cells below the hit resolve for the first move.
                        let plan = planned(node, &columns, inputs, &state)?;
                        for (position, wanted) in cut_requests(&plan, node.inputs.len())?
                            .into_iter()
                            .enumerate()
                        {
                            let input = node.inputs[position];
                            if nodes.nodes[input].cells {
                                requests[input].extend(wanted);
                                needed[input] = true;
                            }
                        }
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
            }
            store.next_flight += 1;
            let id = store.next_flight;
            let (result, sender, abort) = start(
                inner,
                &state,
                keys[index].clone().map(|key| (key, id)),
                columns.iter().copied().collect(),
                widened,
            );
            if let Some(key) = &keys[index] {
                // A narrower computation that no run waits for stops.
                let narrower: Vec<Arc<Flight>> = (store.flights.get(key).into_iter().flatten())
                    .filter(|flight| *lock(&flight.waiters) == 0)
                    .cloned()
                    .collect();
                for flight in narrower {
                    flight.abort.abort();
                    store.land(key, flight.id);
                }
                let flight = Arc::new(Flight {
                    id,
                    columns: columns.clone(),
                    result: result.clone(),
                    abort: abort.clone(),
                    waiters: std::sync::Mutex::new(1),
                    graph: inner.id,
                });
                store
                    .flights
                    .entry(key.clone())
                    .or_default()
                    .push(flight.clone());
                waiting.push(Waiting(flight));
            }
            drop(store);
            aborts.push((keys[index].clone(), id, abort));
            let plan = planned(node, &columns, inputs, &state)?;
            for (position, wanted) in cut_requests(&plan, node.inputs.len())?
                .into_iter()
                .enumerate()
            {
                let input = node.inputs[position];
                requests[input].extend(wanted);
                needed[input] = true;
            }
            resolutions[index] = Resolution::Computing {
                result,
                job: Some((sender, plan)),
            };
        }
        Ok(())
    })();
    if let Err(error) = failure {
        // The computations that this run started stop and leave the store, so no other run
        // waits for them.
        let mut store = inner.store.state();
        for (key, id, abort) in &aborts {
            abort.abort();
            if let Some(key) = key {
                store.land(key, *id);
            }
        }
        return Err(error);
    }

    // The run has computed its keys: the graph's computations that no run waits for stop.
    {
        let mut store = inner.store.state();
        let idle: Vec<(NodeKey, Arc<Flight>)> = (store.flights.iter())
            .flat_map(|(key, flights)| {
                flights
                    .iter()
                    .map(move |flight| (key.clone(), flight.clone()))
            })
            .filter(|(_, flight)| flight.graph == inner.id && *lock(&flight.waiters) == 0)
            .collect();
        for (key, flight) in idle {
            flight.abort.abort();
            store.land(&key, flight.id);
        }
    }

    // The second pass: give each computation the results of the nodes below it.
    for index in 0..count {
        let Resolution::Computing { job, .. } = &mut resolutions[index] else {
            continue;
        };
        let Some((sender, plan)) = job.take() else {
            continue;
        };
        let node = &nodes.nodes[index];
        let job_inputs = (node.inputs.iter())
            .map(|input| input_of(&resolutions[*input]))
            .collect();
        let _ = sender.send(Job {
            plan,
            inputs: job_inputs,
        });
    }

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
            Resolution::Computing { result, .. } => match result.clone().await {
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
