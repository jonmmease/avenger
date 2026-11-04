//! Graphs: the plans of a document, cut into nodes.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use avenger_selection_datafusion::{
    preaggregate::{prepare, Focus, Strategy},
    SelectionRead,
};
use datafusion::{
    catalog::TableProvider,
    common::{
        plan_err,
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Result, TableReference,
    },
    datasource::source_as_provider,
    execution::SessionState,
    logical_expr::LogicalPlan,
};

use crate::{
    cut::{cut_scan, CutSource},
    key::{address, settings_hash, Identity},
    reads::Reads,
    run::{Inputs, Run, RunKey},
    split::split_filters,
    store::{lock, resolve, ResultStore, Source},
};

/// The number of foci whose variants a graph keeps.
const VARIANTS: usize = 4;

/// The id of a graph in its store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GraphId(pub u64);

/// The plans of one document, cut into one graph of nodes. Clones share the graph.
#[derive(Clone)]
pub struct QueryGraph {
    pub(crate) inner: Arc<GraphInner>,
}

pub(crate) struct GraphInner {
    pub id: GraphId,
    pub store: ResultStore,
    pub state: SessionState,
    pub settings: u64,
    /// The plans as `compile` took them, which a focus prepares.
    pub plans: Vec<LogicalPlan>,
    pub direct: Arc<Nodes>,
    pub variants: Mutex<Vec<(Focus, Arc<Nodes>)>>,
}

/// The nodes of a graph's direct plans or of a variant, in an order where each node follows the
/// nodes that it reads, with the top node of each plan.
pub(crate) struct Nodes {
    pub nodes: Arc<[QueryNode]>,
    pub tops: Vec<usize>,
}

/// A node: its plan, with each cut as a scan, the inputs that its own operators read, the nodes
/// that it reads, whether its result is kept, and the aggregates that its rollups replace.
#[derive(Clone, Debug)]
pub struct QueryNode {
    pub(crate) plan: LogicalPlan,
    pub(crate) identity: Identity,
    /// What the node's subplan reads, with the nodes below it.
    pub(crate) reads: Reads,
    pub(crate) params: Vec<String>,
    pub(crate) selections: Vec<SelectionRead>,
    pub(crate) tables: Vec<TableReference>,
    /// The tables that the node's subplan scans, with the nodes below it, and their providers.
    pub(crate) sources: Vec<(TableReference, Arc<dyn TableProvider>)>,
    pub(crate) inputs: Vec<usize>,
    pub(crate) top_of: Vec<usize>,
    pub(crate) kept: bool,
    pub(crate) replaces: Vec<LogicalPlan>,
    /// Whether the node is the cells subplan of a pre-aggregation.
    pub(crate) cells: bool,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Identity")
    }
}

impl QueryNode {
    /// The node's plan, with the scan of each node that it reads named `input_<position>`, by the
    /// position of that node in [`inputs`](Self::inputs).
    pub fn plan(&self) -> &LogicalPlan {
        &self.plan
    }

    /// The placeholders of the parameters that the node's own operators read, such as `$year`.
    pub fn params(&self) -> &[String] {
        &self.params
    }

    /// The clauses of the selections that the node's own operators read.
    pub fn selections(&self) -> &[SelectionRead] {
        &self.selections
    }

    /// The tables that the node's own operators scan.
    pub fn tables(&self) -> &[TableReference] {
        &self.tables
    }

    /// The indices of the nodes that the node reads, in the order of its cuts.
    pub fn inputs(&self) -> &[usize] {
        &self.inputs
    }

    /// The indices of the plans whose top node this node is.
    pub fn top_of(&self) -> &[usize] {
        &self.top_of
    }

    /// Whether the node's result is kept: false when the node, or a node below it, calls a
    /// volatile or a stable function.
    pub fn kept(&self) -> bool {
        self.kept
    }

    /// The aggregates that the node's rollups replace.
    pub fn replaces(&self) -> &[LogicalPlan] {
        &self.replaces
    }
}

impl QueryGraph {
    pub(crate) fn compile(
        store: ResultStore,
        state: &SessionState,
        plans: Vec<LogicalPlan>,
    ) -> Result<Self> {
        if plans.is_empty() {
            return plan_err!("a graph needs a plan");
        }
        let direct = Arc::new(Builder::compile(&store, state, &plans, &[])?);
        let graph = Self {
            inner: Arc::new(GraphInner {
                id: store.graph_id(),
                settings: settings_hash(state),
                state: state.clone(),
                plans,
                direct,
                variants: Mutex::new(Vec::new()),
                store,
            }),
        };
        graph.record_sources();
        Ok(graph)
    }

    /// Record, for each source that the graph scans, its provider and the graph's session, so
    /// the store can check the source.
    fn record_sources(&self) {
        let session = &self.inner.state;
        let mut state = self.inner.store.state();
        for node in self.inner.direct.nodes.iter() {
            for (table, provider) in &node.sources {
                (state.sources.entry(address(provider))).or_insert_with(|| Source {
                    provider: provider.clone(),
                    table: resolve(table, session),
                    session: session.clone(),
                    version: 0,
                    checked: None,
                    files: None,
                });
            }
        }
    }

    /// The nodes of the graph's direct plans, or of its variant for `focus`.
    pub fn nodes(&self, focus: Option<&Focus>) -> Result<Arc<[QueryNode]>> {
        Ok(self.resolved(focus)?.nodes.clone())
    }

    /// The nodes for `focus`: the direct plans' without one, or the variant's.
    pub(crate) fn resolved(&self, focus: Option<&Focus>) -> Result<Arc<Nodes>> {
        let Some(focus) = focus else {
            return Ok(self.inner.direct.clone());
        };
        let mut variants = lock(&self.inner.variants);
        if let Some(position) = variants.iter().position(|(kept, _)| kept == focus) {
            let variant = variants.remove(position);
            let nodes = variant.1.clone();
            variants.push(variant);
            return Ok(nodes);
        }
        let mut substitutions = Vec::new();
        let mut prepared = Vec::new();
        for plan in &self.inner.plans {
            let ready = prepare(plan, focus)?;
            for strategy in &ready.strategies {
                if let Strategy::Preaggregated(preaggregation) = strategy {
                    substitutions.push(Substitution {
                        rollup: split_filters(preaggregation.rollup().clone())?,
                        aggregate: split_filters(preaggregation.aggregate().clone())?,
                        cells: split_filters(preaggregation.cells().clone())?,
                        replaced: preaggregation.aggregate().clone(),
                    });
                }
            }
            prepared.push(ready.plan);
        }
        let nodes = Arc::new(Builder::compile(
            &self.inner.store,
            &self.inner.state,
            &prepared,
            &substitutions,
        )?);
        if variants.len() == VARIANTS {
            variants.remove(0);
        }
        variants.push((focus.clone(), nodes.clone()));
        Ok(nodes)
    }

    /// The key of a run with `inputs`, for a `Task` request. Runs nothing.
    pub fn key(&self, inputs: &Inputs) -> Result<RunKey> {
        crate::run::run_key(self, inputs)
    }

    /// Run the graph with `inputs`, and return the batches of each plan.
    pub async fn run(&self, inputs: &Inputs) -> Result<Run> {
        crate::run::run(self, inputs).await
    }

    /// A Graphviz DOT graph of the nodes of the direct plans, or of the variant for `focus`.
    pub fn to_dot(&self, focus: Option<&Focus>) -> Result<String> {
        Ok(crate::dot::to_dot(&self.nodes(focus)?, None))
    }
}

/// A rollup of a variant, with the aggregate that it replaces and its cells subplan, each with
/// its filters split as the variant's plans are.
pub(crate) struct Substitution {
    rollup: LogicalPlan,
    aggregate: LogicalPlan,
    cells: LogicalPlan,
    /// The aggregate as the plan wrote it.
    replaced: LogicalPlan,
}

/// What the cut rule needs to know of a subplan.
#[derive(Clone)]
struct Info {
    reads: Reads,
    /// Whether the subplan is a scan, alone or under projections and aliases that read no input,
    /// or a list of values.
    scan_like: bool,
    /// Whether the subplan is a scan whose source's scans are materialized.
    materialized: bool,
}

/// The state of a compile: the nodes so far, by identity, and what each subplan reads.
struct Builder<'a> {
    store: &'a ResultStore,
    session: &'a SessionState,
    substitutions: &'a [Substitution],
    nodes: Vec<QueryNode>,
    index: HashMap<Identity, usize>,
    infos: HashMap<*const LogicalPlan, Info>,
}

impl<'a> Builder<'a> {
    fn compile(
        store: &'a ResultStore,
        session: &'a SessionState,
        plans: &[LogicalPlan],
        substitutions: &'a [Substitution],
    ) -> Result<Nodes> {
        let plans: Vec<LogicalPlan> = (plans.iter().cloned())
            .map(split_filters)
            .collect::<Result<_>>()?;
        let mut builder = Builder {
            store,
            session,
            substitutions,
            nodes: Vec::new(),
            index: HashMap::new(),
            infos: HashMap::new(),
        };
        let mut tops = Vec::new();
        for (position, plan) in plans.iter().enumerate() {
            let top = builder.node(plan)?;
            builder.nodes[top].top_of.push(position);
            tops.push(top);
        }
        Ok(Nodes {
            nodes: builder.nodes.into(),
            tops,
        })
    }

    fn info(&mut self, plan: &LogicalPlan) -> Result<Info> {
        let address = plan as *const LogicalPlan;
        if let Some(info) = self.infos.get(&address) {
            return Ok(info.clone());
        }
        let own = Reads::of_operator(plan)?;
        let children = (plan.inputs().into_iter())
            .map(|input| self.info(input))
            .collect::<Result<Vec<_>>>()?;
        let mut reads = own.clone();
        for child in &children {
            reads.add(&child.reads);
        }
        let quiet = own.count() == 0 && !own.volatile;
        let (scan_like, materialized) = match plan {
            LogicalPlan::TableScan(scan) => (
                scan.filters.is_empty() && scan.fetch.is_none(),
                self.store.materialized(&scan.table_name, self.session),
            ),
            LogicalPlan::Projection(_) | LogicalPlan::SubqueryAlias(_) => {
                let child = &children[0];
                (quiet && child.scan_like, child.materialized)
            }
            LogicalPlan::Values(_) | LogicalPlan::EmptyRelation(_) => (true, false),
            _ => (false, false),
        };
        let info = Info {
            reads,
            scan_like,
            materialized,
        };
        self.infos.insert(address, info.clone());
        Ok(info)
    }

    /// Whether the edge from `op` to its input `child` is cut.
    fn cuts(&mut self, op: &LogicalPlan, child: &LogicalPlan) -> Result<bool> {
        let below = self.info(child)?;
        if below.reads.volatile {
            return Ok(false);
        }
        let above = self.info(op)?;
        if below.scan_like {
            return Ok(below.materialized && !above.scan_like);
        }
        Ok(below.reads != above.reads)
    }

    /// The subplan with each rollup replaced by the aggregate that it replaces.
    fn direct_form(&self, plan: &LogicalPlan) -> Result<LogicalPlan> {
        if self.substitutions.is_empty() {
            return Ok(plan.clone());
        }
        Ok(plan
            .clone()
            .transform_down_with_subqueries(|node| {
                match (self.substitutions.iter()).find(|substitution| substitution.rollup == node) {
                    Some(substitution) => Ok(Transformed::new(
                        substitution.aggregate.clone(),
                        true,
                        TreeNodeRecursion::Jump,
                    )),
                    None => Ok(Transformed::no(node)),
                }
            })?
            .data)
    }

    /// The node of the subplan `plan`, which compiles it on first use.
    fn node(&mut self, plan: &LogicalPlan) -> Result<usize> {
        let direct = self.direct_form(plan)?;
        let reads = Reads::of_plan(&direct)?;
        let mut sources = Vec::new();
        direct.apply_with_subqueries(|node| {
            if let LogicalPlan::TableScan(scan) = node {
                let provider = source_as_provider(&scan.source)?;
                sources.push((scan.table_name.clone(), provider));
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        let providers = sources
            .iter()
            .map(|(_, provider)| provider.clone())
            .collect();
        let identity = Identity::new(direct, providers);
        if !reads.volatile {
            if let Some(index) = self.index.get(&identity) {
                return Ok(*index);
            }
        }
        let mut inputs = Vec::new();
        let mut replaces = Vec::new();
        let cut = self.cut_below(plan, &mut inputs, &mut replaces, false)?;
        let own = Reads::of_plan(&cut)?;
        let mut tables = Vec::new();
        cut.apply_with_subqueries(|node| {
            if let LogicalPlan::TableScan(scan) = node {
                if scan.source.as_ref().downcast_ref::<CutSource>().is_none() {
                    tables.push(scan.table_name.clone());
                }
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        let cells = (self.substitutions.iter()).any(|substitution| substitution.cells == *plan);
        let node = QueryNode {
            plan: cut,
            identity: identity.clone(),
            kept: !reads.volatile,
            reads,
            params: own.params.iter().cloned().collect(),
            selections: own.selection_reads(),
            tables,
            sources,
            inputs,
            top_of: Vec::new(),
            replaces,
            cells,
        };
        let index = self.nodes.len();
        if node.kept {
            self.index.insert(identity, index);
        }
        self.nodes.push(node);
        Ok(index)
    }

    /// The operator `op` and the operators below it that are in its node, with each cut below
    /// them replaced by a scan of the cut's node. Nothing inside a recursive query is cut, because
    /// its recursive term reads the work table that the query's execution fills.
    fn cut_below(
        &mut self,
        op: &LogicalPlan,
        inputs: &mut Vec<usize>,
        replaces: &mut Vec<LogicalPlan>,
        recursive: bool,
    ) -> Result<LogicalPlan> {
        if let Some(substitution) = (self.substitutions.iter()).find(|s| s.rollup == *op) {
            replaces.push(substitution.replaced.clone());
        }
        let recursive = recursive || matches!(op, LogicalPlan::RecursiveQuery(_));
        let mut children = Vec::new();
        for child in op.inputs() {
            if !recursive && self.cuts(op, child)? {
                let node = self.node(child)?;
                inputs.push(node);
                children.push(cut_scan(child, inputs.len() - 1)?);
            } else {
                children.push(self.cut_below(child, inputs, replaces, recursive)?);
            }
        }
        if children.is_empty() {
            return Ok(op.clone());
        }
        let mut children = children.into_iter();
        Ok(op
            .clone()
            .map_children(|_| {
                Ok(Transformed::yes(
                    children.next().expect("a child for each input"),
                ))
            })?
            .data)
    }
}
