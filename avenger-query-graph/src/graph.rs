//! Graphs: the plans of a document, cut into nodes.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use avenger_datafusion_shared::SharedNode;
use avenger_selection_datafusion::{SelectionFields, SelectionRead};
use datafusion::{
    catalog::TableProvider,
    common::{
        plan_err,
        tree_node::{Transformed, TransformedResult, TreeNode, TreeNodeRecursion},
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
    run::{Inputs, Memos, Run, RunKey},
    split::split_filters,
    store::{defaults, resolve, ResultStore, Source},
};

/// The id of a graph in its store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GraphId(pub u64);

/// The plans of one document, cut into one graph of nodes. Clones share the graph.
#[derive(Clone)]
pub struct QueryGraph {
    pub(crate) inner: Arc<GraphInner>,
}

/// When its last clone is dropped, a graph stops its computations that nothing waits for.
impl Drop for GraphInner {
    fn drop(&mut self) {
        self.store.dropped(self.id);
    }
}

pub(crate) struct GraphInner {
    pub id: GraphId,
    pub store: ResultStore,
    pub state: SessionState,
    pub settings: u64,
    /// The nodes of the graph's plans, in an order where each node follows the nodes that it
    /// reads.
    pub nodes: Arc<[QueryNode]>,
    /// The top node of each plan.
    pub tops: Vec<usize>,
    /// The columns that each selection's test can read, which the selections' operators of the
    /// nodes' plans read.
    pub fields: SelectionFields,
    /// The plans of the kept nodes that the graph's runs planned, which later runs reuse.
    pub memos: Mutex<Memos>,
    /// Whether the graph is planned for a page: with one partition, and each table read through a
    /// node of its own.
    pub page: bool,
}

/// A node: its plan, with each cut as a scan, the inputs that its own operators read, the nodes
/// that it reads, and whether its result is kept.
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
}

impl QueryGraph {
    pub(crate) fn compile(
        store: ResultStore,
        state: &SessionState,
        plans: Vec<LogicalPlan>,
        fields: SelectionFields,
        page: bool,
    ) -> Result<Self> {
        if plans.is_empty() {
            return plan_err!("a graph needs a plan");
        }
        let (nodes, tops) = Builder::compile(&store, state, &plans, page)?;
        let graph = Self {
            inner: Arc::new(GraphInner {
                id: store.graph_id(),
                settings: settings_hash(state),
                state: state.clone(),
                nodes,
                tops,
                fields,
                memos: Mutex::new(Memos::default()),
                page,
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
        for node in self.inner.nodes.iter() {
            for (table, provider) in &node.sources {
                let source = (state.sources.entry(address(provider))).or_insert_with(|| Source {
                    provider: provider.clone(),
                    names: Vec::new(),
                    version: 0,
                    checked: None,
                    files: None,
                });
                // A name keeps each session whose defaults resolve references differently, so
                // `table_changed` and options resolve a reference as each session does.
                let table = resolve(table, session);
                let known = (source.names.iter())
                    .any(|(name, known)| *name == table && defaults(known) == defaults(session));
                if !known {
                    source.names.push((table, session.clone()));
                }
            }
        }
    }

    /// The nodes of the graph's plans.
    pub fn nodes(&self) -> Arc<[QueryNode]> {
        self.inner.nodes.clone()
    }

    /// The key of a run with `inputs`, for a `Task` request. Runs nothing.
    pub fn key(&self, inputs: &Inputs) -> Result<RunKey> {
        crate::run::run_key(self, inputs)
    }

    /// Run the graph with `inputs`, and return the batches of each plan.
    pub async fn run(&self, inputs: &Inputs) -> Result<Run> {
        crate::run::run(self, inputs).await
    }

    /// A Graphviz DOT graph of the nodes of the graph's plans.
    pub fn to_dot(&self) -> String {
        crate::dot::to_dot(&self.nodes(), None)
    }
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
    /// Whether every scan is a node of its own, as for a page.
    cut_scans: bool,
    /// The inputs that [`unshare`] took out of the plans' shared nodes. Each is cut into a graph
    /// node of its own, so every reader of one input reads the same result.
    shared: HashSet<LogicalPlan>,
    nodes: Vec<QueryNode>,
    index: HashMap<Identity, usize>,
    infos: HashMap<*const LogicalPlan, Info>,
    /// The identities of the store's kept results and computations when the compile began.
    known: HashSet<Identity>,
}

impl<'a> Builder<'a> {
    fn compile(
        store: &'a ResultStore,
        session: &'a SessionState,
        plans: &[LogicalPlan],
        cut_scans: bool,
    ) -> Result<(Arc<[QueryNode]>, Vec<usize>)> {
        let mut shared = HashSet::new();
        let plans: Vec<LogicalPlan> = (plans.iter().cloned())
            .map(|plan| unshare(split_filters(plan)?, &mut shared))
            .collect::<Result<_>>()?;
        let mut builder = Builder {
            store,
            session,
            shared,
            cut_scans,
            nodes: Vec::new(),
            index: HashMap::new(),
            infos: HashMap::new(),
            known: store.identities(),
        };
        let mut tops = Vec::new();
        for (position, plan) in plans.iter().enumerate() {
            let top = builder.node(plan)?;
            builder.nodes[top].top_of.push(position);
            tops.push(top);
        }
        Ok((builder.nodes.into(), tops))
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
                self.cut_scans || self.store.materialized(&scan.table_name, self.session),
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
        if self.shared.contains(child) {
            return Ok(true);
        }
        let above = self.info(op)?;
        if below.scan_like {
            return Ok(below.materialized && !above.scan_like);
        }
        if below.reads != above.reads {
            return Ok(true);
        }
        // A subplan whose result the store keeps, or computes, is a node of its own, so a plan
        // that a reloaded document builds reuses the result where its edit leaves the subplan.
        if self.known.is_empty() {
            return Ok(false);
        }
        let (_, identity) = self.identify(child)?;
        Ok(self.known.contains(&identity))
    }

    /// The tables that `plan` scans with their providers, and its identity.
    #[allow(clippy::type_complexity)]
    fn identify(
        &self,
        plan: &LogicalPlan,
    ) -> Result<(Vec<(TableReference, Arc<dyn TableProvider>)>, Identity)> {
        let mut sources = Vec::new();
        plan.apply_with_subqueries(|node| {
            if let LogicalPlan::TableScan(scan) = node {
                let provider = source_as_provider(&scan.source)?;
                sources.push((scan.table_name.clone(), provider));
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        let scanned = sources
            .iter()
            .map(|(_, provider)| provider.clone())
            .collect();
        let identity = Identity::new(plan.clone(), scanned);
        Ok((sources, identity))
    }

    /// The node of the subplan `plan`, which compiles it on first use.
    fn node(&mut self, plan: &LogicalPlan) -> Result<usize> {
        let (sources, identity) = self.identify(plan)?;
        let reads = Reads::of_plan(plan)?;
        if !reads.volatile {
            if let Some(index) = self.index.get(&identity) {
                return Ok(*index);
            }
        }
        let mut inputs = Vec::new();
        let cut = self.cut_below(plan, &mut inputs, false)?;
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
        recursive: bool,
    ) -> Result<LogicalPlan> {
        let recursive = recursive || matches!(op, LogicalPlan::RecursiveQuery(_));
        let mut children = Vec::new();
        for child in op.inputs() {
            if !recursive && self.cuts(op, child)? {
                let node = self.node(child)?;
                inputs.push(node);
                children.push(cut_scan(child, inputs.len() - 1)?);
            } else {
                children.push(self.cut_below(child, inputs, recursive)?);
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

/// Replaces each shared node of `plan` with its input, and adds the input to `shared`, which the
/// builder cuts into a graph node of its own. A shared node whose input calls a volatile or a
/// stable function stays as it is.
///
/// A shared node runs its input once for all the references to it in one plan. The graph gives
/// the same guarantee, because it runs a node once for all the node's readers. The graph needs
/// the input as a node of its own: a `SharedExec` does not expose the plan of its input as a
/// child, so a run could not bind the selections' operators inside it or point its cuts' leaves
/// at the run's results.
fn unshare(plan: LogicalPlan, shared: &mut HashSet<LogicalPlan>) -> Result<LogicalPlan> {
    plan.transform_up_with_subqueries(|node| {
        let LogicalPlan::Extension(extension) = &node else {
            return Ok(Transformed::no(node));
        };
        let Some(shared_node) = (extension.node.as_any()).downcast_ref::<SharedNode>() else {
            return Ok(Transformed::no(node));
        };
        if Reads::of_plan(&shared_node.input)?.volatile {
            return Ok(Transformed::no(node));
        }
        let input = shared_node.input.clone();
        shared.insert(input.clone());
        Ok(Transformed::yes(input))
    })
    .data()
}
