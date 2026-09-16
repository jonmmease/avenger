use std::collections::{BTreeSet, HashMap};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use chrono::Utc;
use datafusion::{
    arrow::{
        array::UInt64Array,
        compute::take,
        datatypes::SchemaRef,
        record_batch::{RecordBatch, RecordBatchOptions},
    },
    common::{DataFusionError, ScalarValue},
    execution::{context::SessionContext, session_state::SessionStateBuilder, SessionState},
    logical_expr::{execution_props::ExecutionProps, LogicalPlan},
    physical_plan::execute_stream,
};
use futures::{future::BoxFuture, TryStreamExt};
use tokio::sync::Semaphore;

use crate::{
    diagnostics::{
        EvaluationReport, NodeReport, PrepareReport, ScopeEvaluationReport, ScopeReport,
    },
    execution::{bind_plan, GraphQueryPlanner},
    fresh_id,
    graph::{reference::TableRef, GraphDef, NodeKind},
    inputs::InputValue,
    result::InstanceResult,
    Error, Graph, GraphResult, Inputs, InputsBuilder, PartitionKey, Result, ScalarOutput,
    ScopeHandle, ScopeInstance, TableOutput, TableSnapshot,
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

    /// Analyze reachable definitions and partition programs without evaluating functions.
    /// Physical planning remains per computation and instance during each query.
    pub async fn prepare(&self, graph: &Graph) -> Result<PreparedGraph> {
        let graph = graph.inner.clone();
        for scope in &graph.scopes {
            if let Some(handle) = &scope.handle {
                for field in handle.key_schema().fields() {
                    crate::partition::validate_key_type(field.data_type())?;
                }
            }
        }
        let requested = vec![true; graph.outputs.len()];
        let (reachable, _) = demand(&graph, &requested);
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
                .map_err(|source| execution_error(&graph, index, source))?;
            plans[index] = Some(plan);
            nodes.push(NodeReport {
                name: node.name.to_string(),
                scope: graph.scope_name(node.scope),
                dependencies: node
                    .analysis
                    .dependencies
                    .iter()
                    .map(|index| node_name(&graph, *index))
                    .collect(),
                inputs: node
                    .analysis
                    .inputs
                    .iter()
                    .map(|index| graph.inputs[*index].name.to_string())
                    .collect(),
                direct_volatility: node.analysis.direct_volatility,
                reuse_scope: node.analysis.reuse_scope,
            });
        }
        let scopes = graph
            .scopes
            .iter()
            .enumerate()
            .map(|(index, scope)| {
                let mut captures = BTreeSet::new();
                for node in graph.nodes.iter().filter(|node| node.scope == index) {
                    for dependency in &node.analysis.dependencies {
                        if graph.nodes[*dependency].scope != index {
                            captures.insert(node_name(&graph, *dependency));
                        }
                    }
                    for input in &node.analysis.inputs {
                        if graph.inputs[*input].scope != index {
                            captures.insert(format!(
                                "{}::{}",
                                graph.scope_name(graph.inputs[*input].scope),
                                graph.inputs[*input].name
                            ));
                        }
                    }
                }
                ScopeReport {
                    name: graph.scope_name(index),
                    parent: scope.parent.map(|parent| graph.scope_name(parent)),
                    key_schema: scope
                        .handle
                        .as_ref()
                        .map(|handle| handle.key_schema().clone()),
                    captures: captures.into_iter().collect(),
                }
            })
            .collect();
        let report = PrepareReport {
            nodes,
            scopes,
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
        InputsBuilder::new(self.inner.graph.clone())
    }
    pub fn explain(&self) -> PrepareReport {
        self.inner.report.clone()
    }

    /// Materialize root and scoped outputs in one evaluation. Scoped handles select
    /// every discovered instance. Shared ancestors execute once in their defining frame.
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
        let (_, scope_needed) = demand(graph, &requested);
        let runtime = &self.inner.runtime;
        let report = EvaluationReport {
            evaluation_id: fresh_id(),
            query_start_time: Utc::now(),
            executed_nodes: vec![],
            physical_plans: 0,
            materialized_bytes: 0,
            scopes: graph
                .scopes
                .iter()
                .enumerate()
                .map(|(scope, _)| ScopeEvaluationReport {
                    name: graph.scope_name(scope),
                    instances: 0,
                    executed_nodes: 0,
                    partitioned_rows: 0,
                })
                .collect(),
        };
        if !requested.iter().any(|requested| *requested) {
            return Ok(GraphResult {
                graph: graph.id,
                root: InstanceResult {
                    scope: 0,
                    instance: None,
                    outputs: HashMap::new(),
                    children: HashMap::new(),
                },
                report,
            });
        }
        let _permit = runtime
            .queries
            .acquire()
            .await
            .expect("private semaphore stays open");
        let mut evaluation = Evaluation {
            prepared: &self.inner,
            inputs,
            requested: &requested,
            scope_needed,
            frames: vec![Frame {
                scope: 0,
                instance: None,
                values: HashMap::new(),
                inputs: HashMap::new(),
            }],
            reservation: Reservation { runtime, bytes: 0 },
            report,
        };
        let root = evaluation.frame().await?;
        evaluation.report.materialized_bytes = evaluation.reservation.bytes;
        Ok(GraphResult {
            graph: graph.id,
            root,
            report: evaluation.report.clone(),
        })
    }
}

fn demand(graph: &GraphDef, requested: &[bool]) -> (Vec<bool>, Vec<bool>) {
    let mut nodes = vec![false; graph.nodes.len()];
    let mut scopes = vec![false; graph.scopes.len()];
    for (index, output) in graph.outputs.iter().enumerate() {
        if !requested[index] {
            continue;
        }
        nodes[output.node] = true;
        let mut scope = Some(output.scope);
        while let Some(index) = scope {
            scopes[index] = true;
            if let Some(discovery) = graph.scopes[index].discovery {
                nodes[discovery] = true;
            }
            scope = graph.scopes[index].parent;
        }
    }
    for index in (0..graph.nodes.len()).rev() {
        if nodes[index] {
            for dependency in &graph.nodes[index].analysis.dependencies {
                nodes[*dependency] = true;
            }
        }
    }
    (nodes, scopes)
}

struct Frame {
    scope: usize,
    instance: Option<ScopeInstance>,
    values: HashMap<usize, InputValue>,
    inputs: HashMap<usize, InputValue>,
}

struct Evaluation<'a> {
    prepared: &'a PreparedInner,
    inputs: &'a Inputs,
    requested: &'a [bool],
    scope_needed: Vec<bool>,
    frames: Vec<Frame>,
    reservation: Reservation<'a>,
    report: EvaluationReport,
}

impl Evaluation<'_> {
    fn frame_index(&self, scope: usize) -> usize {
        self.frames
            .iter()
            .rposition(|frame| frame.scope == scope)
            .expect("validated ancestor frame")
    }

    fn frame(&mut self) -> BoxFuture<'_, Result<InstanceResult>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            let frame_index = self.frames.len() - 1;
            let scope = self.frames[frame_index].scope;
            self.report.scopes[scope].instances += 1;
            self.reservation
                .charge(std::mem::size_of::<Frame>() + std::mem::size_of::<InstanceResult>())
                .map_err(|error| contextual(self.frames[frame_index].instance.as_ref(), error))?;
            let graph = self.prepared.graph.clone();
            let mut result = InstanceResult {
                scope,
                instance: self.frames[frame_index].instance.clone(),
                outputs: HashMap::new(),
                children: HashMap::new(),
            };
            for (index, output) in graph.outputs.iter().enumerate() {
                if output.scope == scope && self.requested[index] {
                    self.node(output.node).await?;
                    let value = &self.frames[frame_index].values[&output.node];
                    let charge = match value {
                        InputValue::Scalar(value) => value.size(),
                        InputValue::Table(_) => 0,
                    };
                    self.reservation.charge(128 + charge).map_err(|error| {
                        contextual(self.frames[frame_index].instance.as_ref(), error)
                    })?;
                    result.outputs.insert(index, value.clone());
                }
            }
            for (child, definition) in graph.scopes.iter().enumerate() {
                if definition.parent != Some(scope) || !self.scope_needed[child] {
                    continue;
                }
                let discovery = definition.discovery.expect("child discovery");
                self.node(discovery).await?;
                let InputValue::Table(table) = self.frames[frame_index].values[&discovery].clone()
                else {
                    unreachable!()
                };
                let handle = definition.handle.as_ref().expect("child handle");
                let rows = definition.rows.as_ref().expect("child rows");
                let TableRef::Node(rows_index) = rows.read.source else {
                    unreachable!()
                };
                let schema = Arc::new(rows.schema().as_arrow().clone());
                let width = schema.fields().len();
                let groups = self
                    .partition(&table, handle, width)
                    .await
                    .map_err(|error| {
                        contextual(self.frames[frame_index].instance.as_ref(), error)
                    })?;
                self.report.scopes[child].partitioned_rows += table.num_rows();
                let mut children = HashMap::new();
                for (key, indices) in groups {
                    let address = match &self.frames[frame_index].instance {
                        Some(parent) => parent.child(handle, key.values().iter().cloned())?,
                        None => handle.instance(key.values().iter().cloned())?,
                    };
                    self.reservation
                        .charge(address.size() + key.size() + 256)
                        .map_err(|error| contextual(Some(&address), error))?;
                    let local = self
                        .materialize_partition(&table, schema.clone(), &indices)
                        .map_err(|error| contextual(Some(&address), error))?;
                    self.frames.push(Frame {
                        scope: child,
                        instance: Some(address),
                        values: HashMap::from([(rows_index, InputValue::Table(local))]),
                        inputs: HashMap::new(),
                    });
                    let child_result = self.frame().await?;
                    self.frames.pop();
                    children.insert(key, child_result);
                }
                self.reservation.charge(128).map_err(|error| {
                    contextual(self.frames[frame_index].instance.as_ref(), error)
                })?;
                result.children.insert(child, children);
            }
            Ok(result)
        })
    }

    fn materialize_partition(
        &mut self,
        table: &TableSnapshot,
        schema: SchemaRef,
        indices: &[(usize, u64)],
    ) -> Result<TableSnapshot> {
        let mut batches = vec![];
        let mut offset = 0;
        while offset < indices.len() {
            let batch_index = indices[offset].0;
            let end = indices[offset..]
                .iter()
                .position(|(batch, _)| *batch != batch_index)
                .map(|length| offset + length)
                .unwrap_or(indices.len());
            self.reservation
                .charge((end - offset).saturating_mul(std::mem::size_of::<u64>()))?;
            let take_indices =
                UInt64Array::from_iter_values(indices[offset..end].iter().map(|(_, row)| *row));
            let batch = &table.batches()[batch_index];
            let columns = batch.columns()[..schema.fields().len()]
                .iter()
                .map(|column| take(column, &take_indices, None))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let batch = RecordBatch::try_new_with_options(
                schema.clone(),
                columns,
                &RecordBatchOptions::new().with_row_count(Some(end - offset)),
            )?;
            self.reservation
                .charge(batch.get_array_memory_size() + std::mem::size_of::<RecordBatch>())?;
            batches.push(batch);
            offset = end;
        }
        TableSnapshot::from_batches(schema, batches)
    }

    async fn partition(
        &mut self,
        table: &TableSnapshot,
        handle: &ScopeHandle,
        width: usize,
    ) -> Result<HashMap<PartitionKey, Vec<(usize, u64)>>> {
        let mut groups: HashMap<PartitionKey, Vec<(usize, u64)>> = HashMap::new();
        for (batch_index, batch) in table.batches().iter().enumerate() {
            for row in 0..batch.num_rows() {
                if row % 4096 == 0 {
                    tokio::task::yield_now().await;
                }
                let values = batch.columns()[width..]
                    .iter()
                    .map(|column| ScalarValue::try_from_array(column, row))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                let key = handle.key(values)?;
                if !groups.contains_key(&key) {
                    self.reservation.charge(256 + key.size())?;
                }
                // Charge growth conservatively, including spare vector capacity.
                self.reservation
                    .charge(2 * std::mem::size_of::<(usize, u64)>())?;
                groups
                    .entry(key)
                    .or_default()
                    .push((batch_index, row as u64));
            }
        }
        Ok(groups)
    }

    fn node(&mut self, index: usize) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let graph = self.prepared.graph.clone();
            let node = &graph.nodes[index];
            let frame = self.frame_index(node.scope);
            if self.frames[frame].values.contains_key(&index) {
                return Ok(());
            }
            assert!(!node.local_rows, "local rows installed at frame creation");
            for dependency in &node.analysis.dependencies {
                self.node(*dependency).await?;
            }
            for input in &node.analysis.inputs {
                let owner = self.frame_index(graph.inputs[*input].scope);
                if self.frames[owner].inputs.contains_key(input) {
                    continue;
                }
                let address = self.frames[owner].instance.as_ref();
                let value = self.inputs.resolve(*input, address).ok_or_else(|| {
                    contextual(
                        address,
                        Error::MissingInput(graph.inputs[*input].name.to_string()),
                    )
                })?;
                let charge = match value {
                    InputValue::Scalar(value) => value.size(),
                    InputValue::Table(_) => 0,
                };
                self.reservation
                    .charge(128 + charge)
                    .map_err(|error| contextual(address, error))?;
                self.frames[owner].inputs.insert(*input, value.clone());
            }
            let value = self
                .execute(index)
                .await
                .map_err(|error| contextual(self.frames[frame].instance.as_ref(), error))?;
            tokio::task::yield_now().await;
            self.reservation
                .charge(128)
                .map_err(|error| contextual(self.frames[frame].instance.as_ref(), error))?;
            self.frames[frame].values.insert(index, value);
            self.report.executed_nodes.push(node_name(&graph, index));
            self.report.scopes[node.scope].executed_nodes += 1;
            Ok(())
        })
    }

    async fn execute(&mut self, index: usize) -> Result<InputValue> {
        let graph = &self.prepared.graph;
        let node = &graph.nodes[index];
        let plan = bind_plan(
            self.prepared.plans[index]
                .as_ref()
                .expect("reachable node prepared")
                .clone(),
            graph,
            |index| &self.frames[self.frame_index(graph.inputs[index].scope)].inputs[&index],
            |index| &self.frames[self.frame_index(graph.nodes[index].scope)].values[&index],
        )
        .map_err(|source| execution_error(graph, index, source))?;
        let mut state = self.prepared.runtime.state.clone();
        let mut properties =
            ExecutionProps::new().with_query_execution_start_time(self.report.query_start_time);
        properties.config_options = Some(state.config_options().clone());
        *state.execution_props_mut() = properties;
        let physical = state
            .create_physical_plan(&plan)
            .await
            .map_err(|source| execution_error(graph, index, source))?;
        self.report.physical_plans += 1;
        let mut stream = execute_stream(physical, state.task_ctx())
            .map_err(|source| execution_error(graph, index, source))?;
        let schema = Arc::new(node.plan.schema().as_arrow().clone());
        let mut batches = vec![];
        while let Some(batch) = stream
            .try_next()
            .await
            .map_err(|source| execution_error(graph, index, source))?
        {
            self.reservation.charge(
                batch
                    .get_array_memory_size()
                    .saturating_add(std::mem::size_of::<RecordBatch>()),
            )?;
            // Binding can refine nullability. Public schemas remain fixed.
            let batch = RecordBatch::try_new_with_options(
                schema.clone(),
                batch.columns().to_vec(),
                &RecordBatchOptions::new().with_row_count(Some(batch.num_rows())),
            )?;
            batches.push(batch);
        }
        match node.kind {
            NodeKind::Table => Ok(InputValue::Table(TableSnapshot::from_batches(
                schema, batches,
            )?)),
            NodeKind::Scalar => {
                let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
                if rows != 1 {
                    return Err(execution_error(
                        graph,
                        index,
                        DataFusionError::Execution(format!(
                            "standalone expression produced {rows} rows instead of one"
                        )),
                    ));
                }
                let batch = batches
                    .iter()
                    .find(|batch| batch.num_rows() != 0)
                    .expect("one scalar row");
                let scalar = ScalarValue::try_from_array(batch.column(0), 0)?;
                self.reservation.charge(scalar.size())?;
                Ok(InputValue::Scalar(scalar))
            }
        }
    }
}

fn contextual(instance: Option<&ScopeInstance>, error: Error) -> Error {
    if matches!(error, Error::Scoped { .. }) {
        return error;
    }
    match instance {
        Some(instance) => Error::Scoped {
            instance: instance.clone(),
            source: Box::new(error),
        },
        None => error,
    }
}
fn node_name(graph: &GraphDef, index: usize) -> String {
    let node = &graph.nodes[index];
    if node.scope == 0 {
        node.name.to_string()
    } else {
        format!("{}::{}", graph.scope_name(node.scope), node.name)
    }
}
fn execution_error(graph: &GraphDef, index: usize, source: DataFusionError) -> Error {
    Error::Execution {
        node: node_name(graph, index),
        source,
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
