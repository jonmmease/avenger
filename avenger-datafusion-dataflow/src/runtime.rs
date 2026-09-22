use std::collections::{BTreeSet, HashMap};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
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
use futures::{future::BoxFuture, FutureExt, TryStreamExt};

use tokio::sync::Semaphore;

mod cache_aware;
mod execution_lifetime;
pub(crate) mod warming;

use crate::{
    diagnostics::{
        EvaluationReport, NodeReport, PrepareReport, ScopeEvaluationReport, ScopeReport,
    },
    execution::{bind_plan, GraphQueryPlanner},
    fresh_id,
    graph::{reference::TableRef, GraphDef, NodeKind},
    inputs::{InputBinding, MaterializedValue},
    result::InstanceResult,
    Dataflow, DataflowResult, Error, Inputs, InputsBuilder, PartitionKey, Result, ScalarOutput,
    ScopeHandle, ScopeInstance, TableOutput, TableSnapshot,
};

/// Execution limits and completed-result retention for one runtime.
#[derive(Clone, Debug, Default)]
pub struct RuntimeConfig {
    pub execution: ExecutionConfig,
    pub cache: crate::CachePolicy,
    pub function_versions: std::collections::BTreeMap<String, String>,
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

pub(crate) struct RuntimeInner {
    pub(crate) state: SessionState,
    pub(crate) config: RuntimeConfig,
    queries: Semaphore,
    warming: Arc<warming::Scheduler>,
    active_bytes: AtomicUsize,
    pub(crate) cache: Mutex<crate::cache::Cache>,
    pub(crate) codec: Arc<dyn crate::LogicalExtensionCodec>,
}

#[derive(Clone)]
pub struct Runtime {
    pub(crate) inner: Arc<RuntimeInner>,
}

impl Runtime {
    /// Capture a function registry and application codecs used during native decoding.
    pub fn with_session_state_and_codec(
        state: SessionState,
        config: RuntimeConfig,
        codec: Arc<dyn crate::LogicalExtensionCodec>,
    ) -> Result<Self> {
        let mut runtime = Self::with_session_state(state, config)?;
        Arc::get_mut(&mut runtime.inner).expect("new runtime").codec = codec;
        Ok(runtime)
    }
    pub(crate) fn check_semantics(&self, semantics: &crate::SemanticConfig) -> Result<()> {
        let zone = self
            .inner
            .state
            .config_options()
            .execution
            .time_zone
            .as_deref()
            .unwrap_or("UTC");
        if zone != semantics.time_zone {
            return Err(Error::InvalidConfig(format!(
                "dataflow requires time zone {}, runtime has {zone}",
                semantics.time_zone
            )));
        }
        for (name, version) in &semantics.function_versions {
            if self.inner.config.function_versions.get(name) != Some(version) {
                return Err(Error::InvalidConfig(format!(
                    "missing or incompatible function version {name}={version}"
                )));
            }
        }
        Ok(())
    }

    /// Inspect retained charges shared by this runtime's prepared dataflows.
    pub fn cache_stats(&self) -> crate::CacheStats {
        self.inner.cache.lock().expect("cache lock").stats()
    }
    pub fn new(config: RuntimeConfig) -> Result<Self> {
        Self::with_session_state(SessionContext::new().state(), config)
    }

    /// Capture the planning environment and install the graph's snapshot planner.
    /// External providers must be finite and stable for each prepared lifetime.
    pub fn with_session_state(state: SessionState, config: RuntimeConfig) -> Result<Self> {
        if config.execution.max_active_queries == 0
            || config.execution.max_active_queries > Semaphore::MAX_PERMITS
            || config.execution.max_materialized_bytes == 0
        {
            return Err(Error::InvalidConfig("query concurrency and materialization limits must be positive, with concurrency within Tokio's semaphore limit".into()));
        }
        if let crate::CachePolicy::Lru(cache) = &config.cache {
            if cache.max_bytes == 0 || cache.max_entries == 0 {
                return Err(Error::InvalidConfig("cache limits must be positive".into()));
            }
        }
        let state = SessionStateBuilder::new_from_existing(state)
            .with_query_planner(Arc::new(GraphQueryPlanner))
            .build();
        Ok(Self {
            inner: Arc::new(RuntimeInner {
                queries: Semaphore::new(config.execution.max_active_queries),
                warming: Arc::new(warming::Scheduler::default()),
                state,
                codec: Arc::new(crate::DefaultLogicalExtensionCodec {}),
                cache: Mutex::new(crate::cache::Cache::new(config.cache.clone())),
                config,
                active_bytes: AtomicUsize::new(0),
            }),
        })
    }

    /// Analyze reachable definitions and partition programs without evaluating functions.
    /// Physical planning remains per computation and instance during each query.
    pub async fn prepare(&self, graph: &Dataflow) -> Result<PreparedDataflow> {
        if graph.inner.base.is_some() {
            return Err(Error::BaseRequired);
        }
        self.prepare_program(graph, None).await
    }

    async fn prepare_program(
        &self,
        graph: &Dataflow,
        base: Option<&PreparedInner>,
    ) -> Result<PreparedDataflow> {
        self.check_semantics(&graph.inner.semantics)?;
        let mut graph = (*graph.inner).clone();
        for scope in &graph.scopes {
            if let Some(handle) = &scope.handle {
                for field in handle.key_schema().fields() {
                    crate::partition::validate_key_type(field.data_type())?;
                }
            }
        }
        let expr_sites = crate::expr_input::collect_sites(&graph, false)?;
        let base_expr_sites = crate::expr_input::collect_sites(&graph, true)?;
        let requested = vec![true; graph.outputs.len()];
        let (reachable, _) = demand(&graph, &requested);
        let mut plans = vec![None; graph.nodes.len()];
        let mut nodes = vec![];
        for index in 0..graph.nodes.len() {
            let mut linked = crate::graph::analysis::analyze(
                &graph.nodes[index].plan,
                &graph,
                graph.nodes[index].scope,
            )?;
            link_base(&mut linked, base);
            graph.nodes[index].analysis = linked;
            let node = &graph.nodes[index];
            if !reachable[index] {
                continue;
            }
            let plan = if node
                .analysis
                .inputs
                .iter()
                .any(|i| matches!(graph.inputs[*i].kind, crate::graph::InputKind::Expr(_)))
                || base.is_some_and(|base| {
                    node.analysis.base_inputs.iter().any(|i| {
                        matches!(base.graph.inputs[*i].kind, crate::graph::InputKind::Expr(_))
                    })
                }) {
                node.plan.clone()
            } else {
                self.inner
                    .state
                    .analyzer()
                    .execute_and_check(
                        node.plan.clone(),
                        self.inner.state.config_options(),
                        |_, _| {},
                    )
                    .map_err(|source| execution_error(&graph, index, source))?
            };
            let mut analysis = crate::graph::analysis::analyze(&plan, &graph, node.scope)?;
            link_base(&mut analysis, base);
            analysis
                .dependencies
                .extend(node.analysis.dependencies.iter().copied());
            analysis.inputs.extend(node.analysis.inputs.iter().copied());
            analysis.base_inputs.extend(&node.analysis.base_inputs);
            analysis.base_outputs.extend(&node.analysis.base_outputs);
            for dependency in &analysis.dependencies {
                analysis
                    .inputs
                    .extend(graph.nodes[*dependency].analysis.inputs.iter().copied());
                if graph.nodes[*dependency].analysis.reuse_scope
                    == crate::ReuseScope::EvaluationLocal
                {
                    analysis.reuse_scope = crate::ReuseScope::EvaluationLocal;
                }
            }
            if node.analysis.reuse_scope == crate::ReuseScope::EvaluationLocal {
                analysis.reuse_scope = crate::ReuseScope::EvaluationLocal;
            }
            if !crate::graph::same_schema(plan.schema(), node.plan.schema()) {
                return Err(Error::SchemaMismatch(node.name.to_string()));
            }
            graph.nodes[index].analysis = analysis;
            let node = &graph.nodes[index];
            plans[index] = Some(plan);
            if graph.imports.contains_key(&index) {
                continue;
            }
            let mut node_report = NodeReport {
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
                schema: Arc::new(node.plan.schema().as_arrow().clone()),
                has_external_source: node.analysis.has_source,
            };
            if let Some(base) = base {
                qualify_node_report(&mut node_report, "additional");
                node_report
                    .dependencies
                    .extend(node.analysis.base_outputs.iter().map(|output| {
                        format!(
                            "base::{}",
                            node_name(&base.graph, base.graph.outputs[*output].node)
                        )
                    }));
                node_report.inputs.extend(
                    node.analysis
                        .base_inputs
                        .iter()
                        .map(|input| format!("base::{}", base.graph.inputs[*input].name)),
                );
            }
            nodes.push(node_report);
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
        let mut report = PrepareReport {
            imports: graph
                .imports
                .iter()
                .map(|(node, output)| {
                    let base = base.expect("validated import base");
                    crate::ImportReport {
                        name: graph.nodes[*node].name.to_string(),
                        output: base.graph.outputs[*output].name.to_string(),
                        producer: node_name(&base.graph, base.graph.outputs[*output].node),
                    }
                })
                .collect(),
            nodes,
            scopes,
            outputs: graph
                .outputs
                .iter()
                .map(|output| output.name.to_string())
                .collect(),
            replans_on_query: true,
            cache_enabled: self.inner.cache.lock().expect("cache lock").enabled(),
        };
        report.imports.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(base) = base {
            for (index, scope) in report.scopes.iter_mut().enumerate() {
                scope.name = format!("additional::{}", scope.name);
                scope.parent = scope
                    .parent
                    .as_ref()
                    .map(|parent| format!("additional::{parent}"));
                let mut captures = scope
                    .captures
                    .iter()
                    .map(|name| format!("additional::{name}"))
                    .collect::<BTreeSet<_>>();
                for node in graph.nodes.iter().filter(|node| node.scope == index) {
                    captures.extend(node.analysis.base_outputs.iter().map(|output| {
                        format!(
                            "base::{}",
                            node_name(&base.graph, base.graph.outputs[*output].node)
                        )
                    }));
                    captures.extend(
                        node.analysis
                            .base_inputs
                            .iter()
                            .map(|input| format!("base::{}", base.graph.inputs[*input].name)),
                    );
                }
                scope.captures = captures.into_iter().collect();
            }
        }
        let namespace = fresh_id();
        self.inner
            .cache
            .lock()
            .expect("cache lock")
            .register(namespace);
        let graph = Arc::new(graph);
        Ok(PreparedDataflow {
            inner: Arc::new(PreparedInner {
                runtime: self.inner.clone(),
                namespace,
                bindings: Arc::new(crate::graph::BindingMetadata {
                    id: graph.id,
                    inputs: graph.inputs.clone(),
                    expr_sites,
                }),
                graph,
                plans,
                base_expr_sites,
                report,
            }),
        })
    }
}

struct PreparedInner {
    namespace: u64,
    runtime: Arc<RuntimeInner>,
    graph: Arc<GraphDef>,
    bindings: Arc<crate::graph::BindingMetadata>,
    plans: Vec<Option<LogicalPlan>>,
    base_expr_sites: Vec<Vec<crate::expr_input::ExprSite>>,
    report: PrepareReport,
}

impl Drop for PreparedInner {
    fn drop(&mut self) {
        self.runtime
            .cache
            .lock()
            .expect("cache lock")
            .remove(self.namespace);
    }
}

#[derive(Clone)]
pub struct PreparedDataflow {
    inner: Arc<PreparedInner>,
}

/// Additional computations attached to one immutable prepared base.
/// Clones share both preparations and their cache namespaces.
#[derive(Clone)]
pub struct PreparedExtension {
    base: PreparedDataflow,
    additional: PreparedDataflow,
}

impl PreparedExtension {
    /// Discover only the additional definition's inputs, outputs, and scopes.
    pub fn interface(&self) -> crate::DataflowInterface {
        self.additional.interface()
    }
    /// Bind inputs declared by the additional definition. Base bindings stay separate.
    pub fn inputs(&self) -> InputsBuilder {
        self.additional.inputs()
    }
    /// Clear retained additional results while preserving the base's cache.
    pub fn clear_results(&self) {
        self.additional.clear_results();
    }
    pub fn explain(&self) -> PrepareReport {
        let mut report = self.additional.explain();
        let graph = &self.base.inner.graph;
        let mut requested = vec![false; graph.outputs.len()];
        let additional = &self.additional.inner.graph;
        let (reachable, _) = demand(additional, &vec![true; additional.outputs.len()]);
        for (index, node) in additional.nodes.iter().enumerate() {
            if reachable[index] {
                for output in &node.analysis.base_outputs {
                    requested[*output] = true;
                }
            }
        }
        let (needed, _) = demand(graph, &requested);
        let names = graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(index, _)| needed[*index])
            .map(|(_, node)| node.name.as_ref())
            .collect::<BTreeSet<_>>();
        for node in &self.base.inner.report.nodes {
            // Imported producers and all their dependencies are root-owned.
            if node.scope != "root" || !names.contains(node.name.as_str()) {
                continue;
            }
            let mut node_report = node.clone();
            qualify_node_report(&mut node_report, "base");
            report.nodes.push(node_report);
        }
        report.scopes.push(ScopeReport {
            name: "base::root".into(),
            parent: None,
            key_schema: None,
            captures: vec![],
        });
        report
    }
    /// Materialize additional outputs using independently owned base and local bindings.
    pub async fn query(
        &self,
        tables: &[TableOutput],
        scalars: &[ScalarOutput],
        base_inputs: &Inputs,
        additional_inputs: &Inputs,
    ) -> Result<DataflowResult> {
        self.additional
            .query_with_base(
                tables,
                scalars,
                additional_inputs,
                Some((&self.base.inner, base_inputs)),
            )
            .await
    }
}

fn qualify_node_report(report: &mut NodeReport, origin: &str) {
    report.name = format!("{origin}::{}", report.name);
    report.scope = format!("{origin}::{}", report.scope);
    report
        .dependencies
        .iter_mut()
        .for_each(|name| *name = format!("{origin}::{name}"));
    report
        .inputs
        .iter_mut()
        .for_each(|name| *name = format!("{origin}::{name}"));
}

fn link_base(analysis: &mut crate::graph::analysis::Analysis, base: Option<&PreparedInner>) {
    if let Some(base) = base {
        for output in &analysis.base_outputs {
            let upstream = &base.graph.nodes[base.graph.outputs[*output].node].analysis;
            analysis.base_inputs.extend(&upstream.inputs);
            if upstream.reuse_scope == crate::ReuseScope::EvaluationLocal {
                analysis.reuse_scope = crate::ReuseScope::EvaluationLocal;
            }
        }
    }
}

impl PreparedDataflow {
    /// Prepare an additional definition once against this exact base preparation.
    pub async fn prepare_extension(
        &self,
        additional_dataflow: &Dataflow,
    ) -> Result<PreparedExtension> {
        let interface = additional_dataflow
            .inner
            .base
            .as_ref()
            .ok_or(Error::BaseRequired)?;
        if interface.inner.id != self.inner.graph.id {
            return Err(Error::ForeignHandle);
        }
        let runtime = Runtime {
            inner: self.inner.runtime.clone(),
        };
        let additional = runtime
            .prepare_program(additional_dataflow, Some(&self.inner))
            .await?;
        Ok(PreparedExtension {
            base: self.clone(),
            additional,
        })
    }

    #[cfg(feature = "json")]
    pub(crate) fn expression_from_sql(
        &self,
        input: &crate::ExprInput,
        sql: &str,
    ) -> Result<datafusion::logical_expr::Expr> {
        if input.graph != self.inner.graph.id {
            return Err(Error::ForeignHandle);
        }
        crate::json::binding_expr(
            sql,
            &self.inner.runtime.state,
            &self.inner.bindings.expr_sites[input.index],
        )
        .map_err(|e| Error::InvalidExprInput {
            name: input.name().into(),
            context: "SQL binding".into(),
            reason: e.to_string(),
        })
    }

    /// Retrieve public typed handles without retaining plan lineage.
    pub fn interface(&self) -> crate::DataflowInterface {
        crate::DataflowInterface::new(&self.inner.graph)
    }
    /// Clear this preparation's retained results. Active requests keep their owned values.
    pub fn clear_results(&self) {
        self.inner
            .runtime
            .cache
            .lock()
            .expect("cache lock")
            .clear(self.inner.namespace);
    }
    pub fn inputs(&self) -> InputsBuilder {
        InputsBuilder::new(self.inner.bindings.clone())
    }
    pub fn explain(&self) -> PrepareReport {
        self.inner.report.clone()
    }

    /// Materialize root and scoped outputs in one evaluation. Scoped handles select
    /// every discovered instance. Shared ancestors execute once in their defining frame.
    /// Overlapping queries share reusable calculations. Dropping this future detaches
    /// its interest and stops calculations only when no other consumers need them.
    pub async fn query(
        &self,
        tables: &[TableOutput],
        scalars: &[ScalarOutput],
        inputs: &Inputs,
    ) -> Result<DataflowResult> {
        self.query_with_base(tables, scalars, inputs, None).await
    }

    async fn query_with_base(
        &self,
        tables: &[TableOutput],
        scalars: &[ScalarOutput],
        inputs: &Inputs,
        base: Option<(&Arc<PreparedInner>, &Inputs)>,
    ) -> Result<DataflowResult> {
        if base.is_some_and(|(prepared, inputs)| prepared.graph.id != inputs.graph.id) {
            return Err(Error::ForeignHandle);
        }
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
        let mut report = evaluation_report(graph, base.is_some());
        if !requested.iter().any(|requested| *requested) {
            return Ok(DataflowResult {
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
        let mut frames = Vec::new();
        let mut base_state = None;
        if let Some((prepared, inputs)) = base {
            frames.push(Frame::root(Origin::Base));
            report.scopes.push(ScopeEvaluationReport {
                name: "base::root".into(),
                instances: 0,
                executed_nodes: 0,
                partitioned_rows: 0,
            });
            base_state = Some(BaseEvaluation {
                prepared: prepared.clone(),
                inputs: inputs.clone(),
                epoch: 0,
                expressions: HashMap::new(),
            });
        }
        frames.push(Frame::root(Origin::Local));
        let epoch = {
            let cache = runtime.cache.lock().expect("cache lock");
            if let Some(base) = &mut base_state {
                base.epoch = cache.epoch(base.prepared.namespace);
            }
            cache.epoch(self.inner.namespace)
        };
        let mut evaluation = Evaluation {
            base: base_state,
            prepared: self.inner.clone(),
            inputs: inputs.clone(),
            requested,
            scope_needed,
            epoch,
            frames,
            reservation: Arc::new(Reservation {
                runtime: runtime.clone(),
                bytes: AtomicUsize::new(0),
            }),
            work: Arc::new(Mutex::new(Work::default())),
            cleanup: vec![],
            report,
        };
        evaluation.validate_base_expressions()?;
        let root = evaluation.frame().await?;
        evaluation.finish_report();
        evaluation.report.materialized_bytes = evaluation.reservation.bytes.load(Ordering::Relaxed);
        evaluation.report.retained_bytes = runtime.cache.lock().expect("cache lock").stats().bytes;
        Ok(DataflowResult {
            graph: graph.id,
            root,
            report: evaluation.report.clone(),
        })
    }
}

fn evaluation_report(graph: &GraphDef, additional: bool) -> EvaluationReport {
    EvaluationReport {
        evaluation_id: fresh_id(),
        query_start_time: Utc::now(),
        executed_nodes: vec![],
        physical_plans: 0,
        cache_hits: 0,
        in_flight_hits: 0,
        cache_misses: 0,
        cache_bypasses: 0,
        source_executions: 0,
        retained_bytes: 0,
        materialized_bytes: 0,
        scopes: graph
            .scopes
            .iter()
            .enumerate()
            .map(|(scope, _)| ScopeEvaluationReport {
                name: if additional {
                    format!("additional::{}", graph.scope_name(scope))
                } else {
                    graph.scope_name(scope)
                },
                instances: 0,
                executed_nodes: 0,
                partitioned_rows: 0,
            })
            .collect(),
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Origin {
    Local,
    Base,
}

#[derive(Clone)]
struct Frame {
    origin: Origin,
    scope: usize,
    instance: Option<ScopeInstance>,
    values: HashMap<usize, Arc<NodeValue>>,
    inputs: HashMap<usize, InputBinding>,
    keys: HashMap<usize, crate::cache::BindingKey>,
}

impl Frame {
    fn root(origin: Origin) -> Self {
        Self {
            origin,
            scope: 0,
            instance: None,
            values: HashMap::new(),
            inputs: HashMap::new(),
            keys: HashMap::new(),
        }
    }
}

#[derive(Clone)]
struct BaseEvaluation {
    prepared: Arc<PreparedInner>,
    inputs: Inputs,
    epoch: u64,
    expressions: HashMap<usize, InputBinding>,
}

struct Evaluation {
    cleanup: Vec<BoxFuture<'static, ()>>,
    base: Option<BaseEvaluation>,
    prepared: Arc<PreparedInner>,
    inputs: Inputs,
    requested: Vec<bool>,
    scope_needed: Vec<bool>,
    epoch: u64,
    frames: Vec<Frame>,
    reservation: Arc<Reservation>,
    work: Arc<Mutex<Work>>,
    report: EvaluationReport,
}

impl Evaluation {
    fn program(&self, origin: Origin) -> &PreparedInner {
        match origin {
            Origin::Local => &self.prepared,
            Origin::Base => &self.base.as_ref().expect("attached base").prepared,
        }
    }
    fn epoch(&self, origin: Origin) -> u64 {
        match origin {
            Origin::Local => self.epoch,
            Origin::Base => self.base.as_ref().expect("attached base").epoch,
        }
    }
    fn scope_report(&self, origin: Origin, scope: usize) -> usize {
        match origin {
            Origin::Local => scope,
            Origin::Base => self.prepared.graph.scopes.len() + scope,
        }
    }
    fn qualified_name(&self, origin: Origin, index: usize) -> String {
        let name = node_name(&self.program(origin).graph, index);
        match (origin, self.base.is_some()) {
            (Origin::Base, _) => format!("base::{name}"),
            (Origin::Local, true) => format!("additional::{name}"),
            _ => name,
        }
    }
    fn validate_base_expressions(&mut self) -> Result<()> {
        let Some(base) = &mut self.base else {
            return Ok(());
        };
        for (index, sites) in self.prepared.base_expr_sites.iter().enumerate() {
            if sites.is_empty() {
                continue;
            }
            let input = &base.prepared.graph.inputs[index];
            let crate::graph::InputKind::Expr(field) = &input.kind else {
                unreachable!()
            };
            let Some(InputBinding::Expr(binding)) = base.inputs.resolve(index, None) else {
                return Err(Error::MissingInput(input.name.to_string()));
            };
            let bound = binding.for_sites(sites, field, &format!("base::{}", input.name))?;
            self.reservation.charge(128 + bound.size())?;
            base.expressions
                .insert(index, InputBinding::Expr(Arc::new(bound)));
        }
        Ok(())
    }
    fn frame_index(&self, origin: Origin, scope: usize) -> usize {
        self.frames
            .iter()
            .rposition(|frame| frame.origin == origin && frame.scope == scope)
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
                    self.node(Origin::Local, output.node).await?;
                    let value = &self.frames[frame_index].values[&output.node].value;
                    let charge = match value {
                        MaterializedValue::Scalar(value) => value.size(),
                        MaterializedValue::Table(_) => 0,
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
                self.node(Origin::Local, discovery).await?;
                let MaterializedValue::Table(table) =
                    self.frames[frame_index].values[&discovery].value.clone()
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
                        origin: Origin::Local,
                        scope: child,
                        instance: Some(address.clone()),
                        values: HashMap::from([(
                            rows_index,
                            Arc::new(NodeValue {
                                namespace: self.prepared.namespace,
                                index: rows_index,
                                instance: Some(address.clone()),
                                value: MaterializedValue::Table(local),
                                dependencies: vec![],
                                budget: self.reservation.clone(),
                            }),
                        )]),
                        inputs: HashMap::new(),
                        keys: HashMap::new(),
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

    fn activate_base_frame(&mut self) -> Result<()> {
        let mut work = self.work.lock().expect("work lock");
        if !work.base_active {
            self.reservation.charge(std::mem::size_of::<Frame>())?;
            work.base_active = true;
        }
        Ok(())
    }

    fn load_inputs(&mut self, origin: Origin, indices: &BTreeSet<usize>) -> Result<()> {
        if origin == Origin::Base && !indices.is_empty() {
            self.activate_base_frame()?;
        }
        let graph = self.program(origin).graph.clone();
        for index in indices {
            let owner = self.frame_index(origin, graph.inputs[*index].scope);
            if self.frames[owner].inputs.contains_key(index) {
                continue;
            }
            let inputs = match origin {
                Origin::Local => &self.inputs,
                Origin::Base => &self.base.as_ref().expect("attached base").inputs,
            };
            let address = self.frames[owner].instance.as_ref();
            let binding = inputs.resolve(*index, address).ok_or_else(|| {
                contextual(
                    address,
                    Error::MissingInput(graph.inputs[*index].name.to_string()),
                )
            })?;
            self.reservation
                .charge(128 + binding.frame_size())
                .map_err(|error| contextual(address, error))?;
            self.frames[owner].inputs.insert(*index, binding.clone());
        }
        Ok(())
    }

    fn input_keys(
        &mut self,
        origin: Origin,
        indices: &BTreeSet<usize>,
    ) -> Result<Vec<(usize, crate::cache::BindingKey)>> {
        let graph = self.program(origin).graph.clone();
        let mut keys = Vec::new();
        for index in indices {
            let owner = self.frame_index(origin, graph.inputs[*index].scope);
            if !self.frames[owner].keys.contains_key(index) {
                let key = self.frames[owner].inputs[index].key()?;
                self.reservation.charge(key.size())?;
                self.frames[owner].keys.insert(*index, key);
            }
            keys.push((*index, self.frames[owner].keys[index].clone()));
        }
        Ok(keys)
    }

    fn value_key(
        &mut self,
        origin: Origin,
        index: usize,
    ) -> Result<Option<crate::cache::ValueKey>> {
        let graph = self.program(origin).graph.clone();
        let node = &graph.nodes[index];
        let frame = self.frame_index(origin, node.scope);
        let eligible = node.analysis.reuse_scope == crate::ReuseScope::Reusable;
        Ok(if eligible {
            let inputs = self.input_keys(origin, &node.analysis.inputs)?;
            let base_inputs = if node.analysis.base_inputs.is_empty() {
                Vec::new()
            } else {
                self.input_keys(Origin::Base, &node.analysis.base_inputs)?
            };
            let upstream = if node.analysis.base_outputs.is_empty() {
                None
            } else {
                let base = self.base.as_ref().expect("attached base");
                Some((base.prepared.namespace, base.epoch))
            };
            Some(crate::cache::ValueKey {
                namespace: self.program(origin).namespace,
                node: index,
                instance: self.frames[frame].instance.clone(),
                inputs,
                base_inputs,
                upstream,
            })
        } else {
            None
        })
    }

    fn node(&mut self, origin: Origin, index: usize) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let graph = self.program(origin).graph.clone();
            let node = &graph.nodes[index];
            let frame = self.frame_index(origin, node.scope);
            if self.frames[frame].values.contains_key(&index) {
                return Ok(());
            }
            if origin == Origin::Base {
                self.activate_base_frame()?;
            }
            if let Some(output) = graph.imports.get(&index) {
                let producer = self
                    .base
                    .as_ref()
                    .expect("attached base")
                    .prepared
                    .graph
                    .outputs[*output]
                    .node;
                self.node(Origin::Base, producer).await?;
                let base_frame = self.frame_index(Origin::Base, 0);
                let value = self.frames[base_frame].values[&producer].clone();
                self.reservation.charge(
                    128 + match &value.value {
                        MaterializedValue::Scalar(v) => v.size(),
                        _ => 0,
                    },
                )?;
                self.frames[frame].values.insert(index, value);
                return Ok(());
            }
            assert!(!node.local_rows, "local rows installed at frame creation");
            if let LogicalPlan::Extension(extension) = &node.plan {
                if let Some(read) = extension
                    .node
                    .as_any()
                    .downcast_ref::<crate::graph::reference::GraphRead>()
                {
                    if let TableRef::Asset(asset) = read.source {
                        let value = self.node_value(
                            origin,
                            index,
                            MaterializedValue::Table(graph.assets[asset].clone()),
                            vec![],
                        );
                        self.frames[frame].values.insert(index, value);
                        return Ok(());
                    }
                }
            }
            self.load_inputs(origin, &node.analysis.inputs)?;
            if !node.analysis.base_inputs.is_empty() {
                self.load_inputs(Origin::Base, &node.analysis.base_inputs)?;
            }
            let key = self.value_key(origin, index)?;
            if let Some(key) = key {
                self.reservation.charge(key.size())?;
                loop {
                    let runtime = self.prepared.runtime.clone();
                    let lookup = runtime.cache.lock().expect("cache lock").lookup(
                        key.clone(),
                        self.epoch(origin),
                        &runtime,
                        self.qualified_name(origin, index),
                    );
                    match lookup {
                        crate::in_flight::Lookup::Ready(value) => {
                            self.reservation.charge(value.size())?;
                            self.work.lock().expect("work lock").cache_hits += 1;
                            let value = self.node_value(origin, index, value, vec![]);
                            self.frames[frame].values.insert(index, value);
                            return Ok(());
                        }
                        crate::in_flight::Lookup::Pending(pending) => {
                            self.cleanup.push(pending.cleanup());
                            {
                                let mut work = self.work.lock().expect("work lock");
                                work.in_flight_hits += 1;
                                if self.program(origin).report.cache_enabled {
                                    work.cache_misses += 1;
                                }
                            }
                            match pending.wait().await? {
                                Some(value) => {
                                    self.install(value)?;
                                    return Ok(());
                                }
                                None => continue,
                            }
                        }
                        crate::in_flight::Lookup::Reserved(reservation, pending) => {
                            self.cleanup.push(pending.cleanup());
                            if self.program(origin).report.cache_enabled {
                                self.work.lock().expect("work lock").cache_misses += 1;
                            }
                            let mut job = match self.fork_node(origin, index) {
                                Ok(job) => job,
                                Err(error) => {
                                    reservation.finish(Err(error), |_| {});
                                    pending.wait().await?;
                                    unreachable!("failed reservation returns an error");
                                }
                            };
                            let task = tokio::spawn(async move {
                                let result = {
                                    let computation = std::panic::AssertUnwindSafe(
                                        job.compute_node(origin, index),
                                    )
                                    .catch_unwind();
                                    tokio::pin!(computation);
                                    tokio::select! {
                                        biased;
                                        result = &mut computation => Some(result),
                                        () = reservation.unwatched() => None,
                                    }
                                };
                                job.settle_execution().await;
                                let work = job.work.clone();
                                let retention = job.program(origin).report.cache_enabled;
                                drop(job);
                                match result {
                                    Some(Ok(result)) => reservation.finish(result, |bypassed| {
                                        if bypassed && retention {
                                            work.lock().expect("work lock").cache_bypasses += 1;
                                        }
                                    }),
                                    None => reservation.cancel(),
                                    Some(Err(_)) => drop(reservation),
                                }
                            });
                            pending.track(task);
                            match pending.wait().await? {
                                Some(value) => {
                                    self.install(value)?;
                                    return Ok(());
                                }
                                None => continue,
                            }
                        }
                    }
                }
            }
            let result = self.compute_node(origin, index).await;
            self.settle_execution().await;
            let value = result?;
            self.frames[frame].values.insert(index, value);
            Ok(())
        })
    }

    fn fork_node(&self, origin: Origin, index: usize) -> Result<Self> {
        let mut needed = std::collections::HashSet::new();
        let mut pending = vec![(origin, index)];
        while let Some((origin, index)) = pending.pop() {
            if !needed.insert((origin, index)) {
                continue;
            }
            let graph = &self.program(origin).graph;
            if let Some(output) = graph.imports.get(&index) {
                pending.push((
                    Origin::Base,
                    self.program(Origin::Base).graph.outputs[*output].node,
                ));
            }
            pending.extend(
                graph.nodes[index]
                    .analysis
                    .dependencies
                    .iter()
                    .map(|node| (origin, *node)),
            );
        }
        let mut frames = Vec::with_capacity(self.frames.len());
        for frame in &self.frames {
            let values = frame
                .values
                .iter()
                .filter(|(index, _)| needed.contains(&(frame.origin, **index)))
                .map(|(index, value)| (*index, value.clone()))
                .collect::<HashMap<_, _>>();
            self.reservation.charge(
                std::mem::size_of::<Frame>()
                    + 128 * (values.len() + frame.inputs.len() + frame.keys.len()),
            )?;
            frames.push(Frame {
                origin: frame.origin,
                scope: frame.scope,
                instance: frame.instance.clone(),
                values,
                inputs: frame.inputs.clone(),
                keys: frame.keys.clone(),
            });
        }
        Ok(Self {
            prepared: self.prepared.clone(),
            base: self.base.clone(),
            inputs: self.inputs.clone(),
            requested: vec![],
            scope_needed: vec![],
            epoch: self.epoch,
            frames,
            reservation: self.reservation.clone(),
            work: self.work.clone(),
            report: self.report.clone(),
            cleanup: vec![],
        })
    }

    fn compute_node(
        &mut self,
        origin: Origin,
        index: usize,
    ) -> BoxFuture<'_, Result<Arc<NodeValue>>> {
        Box::pin(async move {
            let graph = self.program(origin).graph.clone();
            let node = &graph.nodes[index];
            for dependency in &node.analysis.dependencies {
                self.node(origin, *dependency).await?;
            }
            let result = self.execute(origin, index).await;
            let value = result.map_err(|error| {
                contextual(
                    self.frames[self.frame_index(origin, node.scope)]
                        .instance
                        .as_ref(),
                    error,
                )
            })?;
            self.reservation.charge(128).map_err(|error| {
                contextual(
                    self.frames[self.frame_index(origin, node.scope)]
                        .instance
                        .as_ref(),
                    error,
                )
            })?;
            let dependencies = node
                .analysis
                .dependencies
                .iter()
                .map(|dependency| {
                    self.frames[self.frame_index(origin, graph.nodes[*dependency].scope)].values
                        [dependency]
                        .clone()
                })
                .collect();
            let mut work = self.work.lock().expect("work lock");
            work.executed_nodes.push(self.qualified_name(origin, index));
            *work
                .scopes
                .entry(self.scope_report(origin, node.scope))
                .or_default() += 1;
            Ok(self.node_value(origin, index, value, dependencies))
        })
    }

    fn node_value(
        &self,
        origin: Origin,
        index: usize,
        value: MaterializedValue,
        dependencies: Vec<Arc<NodeValue>>,
    ) -> Arc<NodeValue> {
        Arc::new(NodeValue {
            namespace: self.program(origin).namespace,
            index,
            instance: self.frames
                [self.frame_index(origin, self.program(origin).graph.nodes[index].scope)]
            .instance
            .clone(),
            value,
            dependencies,
            budget: self.reservation.clone(),
        })
    }

    fn install(&mut self, value: Arc<NodeValue>) -> Result<()> {
        let origin = if value.namespace == self.prepared.namespace {
            Origin::Local
        } else {
            Origin::Base
        };
        let scope = self.program(origin).graph.nodes[value.index].scope;
        let frame = self.frame_index(origin, scope);
        debug_assert_eq!(self.frames[frame].instance, value.instance);
        if self.frames[frame].values.contains_key(&value.index) {
            return Ok(());
        }
        for dependency in &value.dependencies {
            self.install(dependency.clone())?;
        }
        if !Arc::ptr_eq(&value.budget, &self.reservation) {
            self.reservation.charge(value.value.size() + 128)?;
        }
        self.frames[frame].values.insert(value.index, value);
        Ok(())
    }

    fn finish_report(&mut self) {
        let work = self.work.lock().expect("work lock");
        if work.base_active {
            let scope = self.scope_report(Origin::Base, 0);
            self.report.scopes[scope].instances = 1;
        }
        self.report.executed_nodes = work.executed_nodes.clone();
        self.report.physical_plans = work.physical_plans;
        self.report.source_executions = work.source_executions;
        self.report.cache_hits = work.cache_hits;
        self.report.in_flight_hits = work.in_flight_hits;
        self.report.cache_misses = work.cache_misses;
        self.report.cache_bypasses = work.cache_bypasses;
        for (scope, count) in &work.scopes {
            self.report.scopes[*scope].executed_nodes = *count;
        }
    }

    fn execution_error(&self, origin: Origin, index: usize, source: DataFusionError) -> Error {
        Error::Execution {
            node: self.qualified_name(origin, index),
            source,
        }
    }

    async fn settle_execution(&mut self) {
        while let Some(completion) = self.cleanup.last_mut() {
            // Admission remains held until workers and detached dependency jobs settle.
            let _ = completion.await;
            self.cleanup.pop();
        }
    }

    async fn execute(&mut self, origin: Origin, index: usize) -> Result<MaterializedValue> {
        let graph = self.program(origin).graph.clone();
        let node = &graph.nodes[index];
        let plan = bind_plan(
            self.program(origin).plans[index]
                .as_ref()
                .expect("reachable node prepared")
                .clone(),
            &graph,
            |index| {
                &self.frames[self.frame_index(origin, graph.inputs[index].scope)].inputs[&index]
            },
            |index| {
                &self.frames[self.frame_index(origin, graph.nodes[index].scope)].values[&index]
                    .value
            },
            |index| {
                let base = self.base.as_ref().expect("attached base");
                base.expressions.get(&index).unwrap_or_else(|| {
                    &self.frames[self.frame_index(Origin::Base, 0)].inputs[&index]
                })
            },
            |output| {
                let base = self.base.as_ref().expect("attached base");
                &self.frames[self.frame_index(Origin::Base, 0)].values
                    [&base.prepared.graph.outputs[output].node]
                    .value
            },
        )
        .map_err(|source| self.execution_error(origin, index, source))?;
        let mut state = self.prepared.runtime.state.clone();
        let mut properties =
            ExecutionProps::new().with_query_execution_start_time(self.report.query_start_time);
        properties.config_options = Some(state.config_options().clone());
        *state.execution_props_mut() = properties;
        let physical = state
            .create_physical_plan(&plan)
            .await
            .map_err(|source| self.execution_error(origin, index, source))?;
        self.work.lock().expect("work lock").physical_plans += 1;
        reject_unbounded(&physical)?;
        let schema = Arc::new(node.plan.schema().as_arrow().clone());
        let actual = physical.schema();
        // Planning may narrow nullability, but cannot change the public column contract.
        if actual.fields().len() != schema.fields().len()
            || actual.fields().iter().zip(schema.fields()).any(|(a, e)| {
                a.name() != e.name()
                    || a.data_type() != e.data_type()
                    || (a.is_nullable() && !e.is_nullable())
            })
        {
            return Err(Error::SchemaMismatch(node.name.to_string()));
        }
        if node.analysis.has_source {
            self.work.lock().expect("work lock").source_executions += 1;
        }
        let mut owners = vec![self.prepared.clone()];
        if let Some(base) = &self.base {
            owners.push(base.prepared.clone());
        }
        let (physical, completion) =
            execution_lifetime::track(physical, self.reservation.clone(), owners)?;
        self.cleanup.push(
            async move {
                let _ = completion.await;
            }
            .boxed(),
        );
        let mut stream = execute_stream(physical, state.task_ctx())
            .map_err(|source| self.execution_error(origin, index, source))?;
        let mut batches = vec![];
        while let Some(batch) = stream
            .try_next()
            .await
            .map_err(|source| self.execution_error(origin, index, source))?
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
            NodeKind::Table => Ok(MaterializedValue::Table(TableSnapshot::from_batches(
                schema, batches,
            )?)),
            NodeKind::Scalar => {
                let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
                if rows != 1 {
                    return Err(self.execution_error(
                        origin,
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
                Ok(MaterializedValue::Scalar(scalar))
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

struct Reservation {
    runtime: Arc<RuntimeInner>,
    bytes: AtomicUsize,
}
impl Reservation {
    fn charge(&self, bytes: usize) -> Result<()> {
        let limit = self.runtime.config.execution.max_materialized_bytes;
        self.runtime
            .active_bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|next| *next <= limit)
            })
            .map_err(|_| Error::ResourceExhausted { limit })?;
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
        Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.runtime
            .active_bytes
            .fetch_sub(self.bytes.load(Ordering::Relaxed), Ordering::Relaxed);
    }
}

fn reject_unbounded(plan: &Arc<dyn datafusion::physical_plan::ExecutionPlan>) -> Result<()> {
    use datafusion::physical_plan::{execution_plan::Boundedness, ExecutionPlanProperties};
    if matches!(plan.boundedness(), Boundedness::Unbounded { .. }) {
        return Err(Error::UnsupportedPlan("unbounded physical source".into()));
    }
    for child in plan.children() {
        reject_unbounded(child)?;
    }
    Ok(())
}

#[derive(Default)]
struct Work {
    base_active: bool,
    executed_nodes: Vec<String>,
    physical_plans: usize,
    source_executions: usize,
    cache_hits: usize,
    in_flight_hits: usize,
    cache_misses: usize,
    cache_bypasses: usize,
    scopes: HashMap<usize, usize>,
}

pub(crate) struct NodeValue {
    namespace: u64,
    index: usize,
    instance: Option<ScopeInstance>,
    pub(crate) value: MaterializedValue,
    dependencies: Vec<Arc<NodeValue>>,
    budget: Arc<Reservation>,
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_execution;
