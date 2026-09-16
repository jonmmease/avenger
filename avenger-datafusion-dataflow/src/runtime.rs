use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use chrono::Utc;
use datafusion::{
    arrow::record_batch::{RecordBatch, RecordBatchOptions},
    common::{DataFusionError, ScalarValue},
    execution::{context::SessionContext, session_state::SessionStateBuilder, SessionState},
    logical_expr::{execution_props::ExecutionProps, LogicalPlan},
    physical_plan::execute_stream,
};
use futures::TryStreamExt;
use tokio::sync::Semaphore;

use crate::{
    diagnostics::{EvaluationReport, NodeReport, PrepareReport},
    execution::{bind_plan, GraphQueryPlanner},
    fresh_id,
    graph::{GraphDef, NodeKind},
    inputs::InputValue,
    Error, Graph, Inputs, InputsBuilder, Result, ScalarOutput, TableOutput, TableSnapshot,
};

/// Controls the initial evaluator. Result caching and physical reuse are not yet implemented.
#[derive(Clone, Debug, Default)]
pub struct RuntimeConfig {
    pub execution: ExecutionConfig,
}

#[derive(Clone, Debug)]
pub struct ExecutionConfig {
    pub max_active_queries: usize,
    /// Aggregate conservative charge for values produced by active queries.
    pub max_materialized_bytes: usize,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_active_queries: 4,
            max_materialized_bytes: 256 * 1024 * 1024,
        }
    }
}

struct RuntimeInner {
    state: SessionState,
    config: RuntimeConfig,
    queries: Semaphore,
    active_bytes: AtomicUsize,
}

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl Runtime {
    pub fn new(config: RuntimeConfig) -> Result<Self> {
        Self::with_session_state(SessionContext::new().state(), config)
    }

    /// Capture the planning environment and install the graph's snapshot planner.
    /// External catalog tables must be supplied through declared table inputs.
    pub fn with_session_state(state: SessionState, config: RuntimeConfig) -> Result<Self> {
        if config.execution.max_active_queries == 0
            || config.execution.max_active_queries > Semaphore::MAX_PERMITS
            || config.execution.max_materialized_bytes == 0
        {
            return Err(Error::InvalidConfig("query concurrency and materialization limits must be positive, with concurrency within Tokio's semaphore limit".into()));
        }
        let state = SessionStateBuilder::new_from_existing(state)
            .with_query_planner(Arc::new(GraphQueryPlanner))
            .build();
        Ok(Self {
            inner: Arc::new(RuntimeInner {
                queries: Semaphore::new(config.execution.max_active_queries),
                state,
                config,
                active_bytes: AtomicUsize::new(0),
            }),
        })
    }

    /// Validate and analyze all declared output paths without evaluating functions.
    /// Physical plans are constructed later, once per executed node per query.
    pub async fn prepare(&self, graph: &Graph) -> Result<PreparedGraph> {
        let graph = graph.inner.clone();
        let mut reachable = vec![false; graph.nodes.len()];
        for output in &graph.outputs {
            reachable[output.node] = true;
        }
        demand_dependencies(&graph, &mut reachable);
        let mut plans = vec![None; graph.nodes.len()];
        let mut nodes = vec![];
        for (index, node) in graph.nodes.iter().enumerate() {
            if !reachable[index] {
                continue;
            }
            let plan = self
                .inner
                .state
                .analyzer()
                .execute_and_check(
                    node.plan.clone(),
                    self.inner.state.config_options(),
                    |_, _| {},
                )
                .map_err(|source| Error::Execution {
                    node: node.name.to_string(),
                    source,
                })?;
            plans[index] = Some(plan);
            nodes.push(NodeReport {
                name: node.name.to_string(),
                dependencies: node
                    .analysis
                    .dependencies
                    .iter()
                    .map(|index| graph.nodes[*index].name.to_string())
                    .collect(),
                inputs: node
                    .analysis
                    .inputs
                    .iter()
                    .map(|index| graph.inputs[*index].name.to_string())
                    .collect(),
                direct_volatility: node.analysis.direct_volatility,
                reuse_scope: node.analysis.scope,
            });
        }
        let report = PrepareReport {
            nodes,
            outputs: graph
                .outputs
                .iter()
                .map(|output| output.name.to_string())
                .collect(),
            replans_on_query: true,
        };
        Ok(PreparedGraph {
            inner: Arc::new(PreparedInner {
                runtime: self.inner.clone(),
                graph,
                plans,
                report,
            }),
        })
    }
}

struct PreparedInner {
    runtime: Arc<RuntimeInner>,
    graph: Arc<GraphDef>,
    plans: Vec<Option<LogicalPlan>>,
    report: PrepareReport,
}

#[derive(Clone)]
pub struct PreparedGraph {
    inner: Arc<PreparedInner>,
}

impl PreparedGraph {
    pub fn inputs(&self) -> InputsBuilder {
        InputsBuilder {
            graph: self.inner.graph.clone(),
            values: vec![None; self.inner.graph.inputs.len()],
        }
    }

    pub fn explain(&self) -> PrepareReport {
        self.inner.report.clone()
    }

    /// Evaluate both output sets in one execution. Empty slices select no outputs
    /// of that kind. Shared prerequisites execute once, with no cross-query cache.
    pub async fn query(
        &self,
        tables: &[TableOutput],
        scalars: &[ScalarOutput],
        inputs: &Inputs,
    ) -> Result<GraphResult> {
        let graph = &self.inner.graph;
        if inputs.graph.id != graph.id {
            return Err(Error::ForeignHandle);
        }
        let mut requested = vec![false; graph.outputs.len()];
        for output in tables {
            if output.graph != graph.id {
                return Err(Error::ForeignHandle);
            }
            requested[output.index] = true;
        }
        for output in scalars {
            if output.graph != graph.id {
                return Err(Error::ForeignHandle);
            }
            requested[output.index] = true;
        }
        let mut report = EvaluationReport {
            evaluation_id: fresh_id(),
            query_start_time: Utc::now(),
            executed_nodes: vec![],
            physical_plans: 0,
            materialized_bytes: 0,
        };
        let mut demanded = vec![false; graph.nodes.len()];
        for (index, output) in graph.outputs.iter().enumerate() {
            if requested[index] {
                demanded[output.node] = true;
            }
        }
        demand_dependencies(graph, &mut demanded);
        let runtime = &self.inner.runtime;
        let _permit = runtime
            .queries
            .acquire()
            .await
            .expect("private semaphore stays open");
        let mut reservation = Reservation { runtime, bytes: 0 };
        let mut values: Vec<Option<InputValue>> = vec![None; graph.nodes.len()];
        for (index, node) in graph.nodes.iter().enumerate() {
            if !demanded[index] {
                continue;
            }
            let plan = bind_plan(
                self.inner.plans[index]
                    .as_ref()
                    .expect("reachable node prepared")
                    .clone(),
                graph,
                &inputs.values,
                &values,
            )
            .map_err(|source| Error::Execution {
                node: node.name.to_string(),
                source,
            })?;
            let mut state = runtime.state.clone();
            let mut properties =
                ExecutionProps::new().with_query_execution_start_time(report.query_start_time);
            properties.config_options = Some(state.config_options().clone());
            *state.execution_props_mut() = properties;
            let physical =
                state
                    .create_physical_plan(&plan)
                    .await
                    .map_err(|source| Error::Execution {
                        node: node.name.to_string(),
                        source,
                    })?;
            report.physical_plans += 1;
            let mut stream =
                execute_stream(physical, state.task_ctx()).map_err(|source| Error::Execution {
                    node: node.name.to_string(),
                    source,
                })?;
            let schema = Arc::new(node.plan.schema().as_arrow().clone());
            let mut batches = vec![];
            while let Some(batch) = stream.try_next().await.map_err(|source| Error::Execution {
                node: node.name.to_string(),
                source,
            })? {
                reservation.charge(
                    batch
                        .get_array_memory_size()
                        .saturating_add(std::mem::size_of::<RecordBatch>()),
                )?;
                // Literal binding can refine nullability. Restore the declared
                // output schema while preserving columns and zero-column row counts.
                let batch = RecordBatch::try_new_with_options(
                    schema.clone(),
                    batch.columns().to_vec(),
                    &RecordBatchOptions::new().with_row_count(Some(batch.num_rows())),
                )?;
                batches.push(batch);
            }
            let value = match node.kind {
                NodeKind::Table => InputValue::Table(TableSnapshot::from_batches(schema, batches)?),
                NodeKind::Scalar => {
                    let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
                    if rows != 1 {
                        return Err(Error::Execution {
                            node: node.name.to_string(),
                            source: DataFusionError::Execution(format!(
                                "standalone expression produced {rows} rows instead of one"
                            )),
                        });
                    }
                    let batch = batches
                        .iter()
                        .find(|batch| batch.num_rows() != 0)
                        .expect("one scalar row");
                    let scalar = ScalarValue::try_from_array(batch.column(0), 0)?;
                    reservation.charge(scalar.size())?;
                    InputValue::Scalar(scalar)
                }
            };
            values[index] = Some(value);
            report.executed_nodes.push(node.name.to_string());
        }
        report.materialized_bytes = reservation.bytes;
        let outputs = graph
            .outputs
            .iter()
            .enumerate()
            .map(|(index, output)| {
                requested[index].then(|| {
                    values[output.node]
                        .as_ref()
                        .expect("requested output evaluated")
                        .clone()
                })
            })
            .collect();
        Ok(GraphResult {
            graph: graph.id,
            outputs,
            report,
        })
    }
}

fn demand_dependencies(graph: &GraphDef, demanded: &mut [bool]) {
    for index in (0..graph.nodes.len()).rev() {
        if demanded[index] {
            for dependency in &graph.nodes[index].analysis.dependencies {
                demanded[*dependency] = true;
            }
        }
    }
}

struct Reservation<'a> {
    runtime: &'a RuntimeInner,
    bytes: usize,
}
impl Reservation<'_> {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        let limit = self.runtime.config.execution.max_materialized_bytes;
        self.runtime
            .active_bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|next| *next <= limit)
            })
            .map_err(|_| Error::ResourceExhausted { limit })?;
        self.bytes += bytes;
        Ok(())
    }
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.runtime
            .active_bytes
            .fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub struct GraphResult {
    graph: u64,
    outputs: Vec<Option<InputValue>>,
    report: EvaluationReport,
}

impl GraphResult {
    pub fn table(&self, output: &TableOutput) -> Result<&TableSnapshot> {
        if output.graph != self.graph {
            return Err(Error::ForeignHandle);
        }
        match self.outputs.get(output.index).and_then(Option::as_ref) {
            Some(InputValue::Table(value)) => Ok(value),
            _ => Err(Error::UnrequestedOutput),
        }
    }
    pub fn scalar(&self, output: &ScalarOutput) -> Result<&ScalarValue> {
        if output.graph != self.graph {
            return Err(Error::ForeignHandle);
        }
        match self.outputs.get(output.index).and_then(Option::as_ref) {
            Some(InputValue::Scalar(value)) => Ok(value),
            _ => Err(Error::UnrequestedOutput),
        }
    }
    pub fn report(&self) -> &EvaluationReport {
        &self.report
    }
}
