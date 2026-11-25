//! Shared inputs: a plan that reads one input several times, while the input runs once.
//!
//! [`shared`] wraps an input in a [`SharedNode`], and every reference to that node in a plan reads
//! the same rows. The optimizer still optimizes the input below the node, but the node blocks the
//! pushdown of filters and projections from its readers into its input, because each reader can
//! need different rows and columns.
//!
//! [`InlineShared`] replaces a node with its input when the input is cheap to compute again: a
//! `VALUES` list, an empty relation, or a scan of an in-memory table, under any number of
//! projections, filters, and aliases. An inlined input keeps the pushdown into each reader. A
//! volatile input is never inlined, so its readers see one set of values.
//!
//! [`PruneShared`] narrows each node's input to the union of the columns that its readers read,
//! after DataFusion's pushdown of projections, so the input computes and buffers no column that
//! no reader reads.
//!
//! [`SharedExtensionPlanner`] plans the references to one node as [`SharedExec`]s of one shared
//! state, which holds the input, optimized once. The first reader of an output partition to run
//! drives the input partition, and the state buffers its batches, under a memory reservation, for
//! the other readers. A batch is freed when every reader has read it.

use std::any::Any;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use datafusion::arrow::array::RecordBatch;
use datafusion::catalog::{MemTable, Session};
use datafusion::common::tree_node::{Transformed, TreeNode, TreeNodeRecursion};
use datafusion::common::{Column, DFSchemaRef, Result};
use datafusion::datasource::source_as_provider;
use datafusion::execution::memory_pool::{MemoryConsumer, MemoryReservation};
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::physical_planning_context::PhysicalPlanningContext;
use datafusion::logical_expr::{
    Expr, Extension, LogicalPlan, Projection, UserDefinedLogicalNode, UserDefinedLogicalNodeCore,
};
use datafusion::optimizer::{ApplyOrder, OptimizerConfig, OptimizerRule};
use datafusion::physical_expr::PhysicalExpr;
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, PlanProperties, ReplaceChildrenOptions,
};
use datafusion::physical_planner::{DefaultPhysicalPlanner, ExtensionPlanner, PhysicalPlanner};
use futures::StreamExt;
use futures::lock::Mutex as AsyncMutex;

// ---------------------------------------------------------------------------------------
// The logical node
// ---------------------------------------------------------------------------------------

/// An input that its readers share. The references to one node have one `id`, a hash of the
/// input when it was made, so equal inputs give equal plans: two analyses of one script plan
/// equally, as a cache of results keyed by their plans needs. A volatile input, such as one that
/// calls `random()`, takes an id of its own instead, so two shared copies of it never run as one.
///
/// The node keeps the schema of its input when it was made. A rule that rebuilds the node over
/// an input with more columns, as DataFusion's pushdown of struct fields toward the leaves does,
/// finds its columns missing from the node and leaves the node as it was, so every reference
/// reads one input.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SharedNode {
    pub id: u64,
    pub input: LogicalPlan,
    schema: DFSchemaRef,
}

/// `input` as a shared node, whose references in a plan read one run of `input`.
pub fn shared(input: LogicalPlan) -> LogicalPlan {
    let id = shared_id(&input);
    let schema = Arc::clone(input.schema());
    LogicalPlan::Extension(Extension {
        node: Arc::new(SharedNode { id, input, schema }),
    })
}

/// The id of a shared node of `input`: the hash of the input, or for a volatile input, the next
/// of a counter of the process, which no hash takes since it sets the high bit.
fn shared_id(input: &LogicalPlan) -> u64 {
    use std::hash::{Hash, Hasher};
    const VOLATILE: u64 = 1 << 63;
    let volatile = input
        .exists(|node| Ok(node.expressions().iter().any(Expr::is_volatile)))
        .unwrap_or(true);
    if volatile {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        return VOLATILE | NEXT.fetch_add(1, AtomicOrdering::Relaxed);
    }
    let mut hasher = std::hash::DefaultHasher::new();
    input.hash(&mut hasher);
    hasher.finish() & !VOLATILE
}

impl PartialOrd for SharedNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.id.partial_cmp(&other.id)
    }
}

impl UserDefinedLogicalNodeCore for SharedNode {
    fn name(&self) -> &str {
        "Shared"
    }
    fn inputs(&self) -> Vec<&LogicalPlan> {
        vec![&self.input]
    }
    fn schema(&self) -> &DFSchemaRef {
        &self.schema
    }
    fn expressions(&self) -> Vec<Expr> {
        vec![]
    }
    fn fmt_for_explain(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Shared: id={}", self.id)
    }
    fn with_exprs_and_inputs(&self, _exprs: Vec<Expr>, inputs: Vec<LogicalPlan>) -> Result<Self> {
        let input = inputs
            .into_iter()
            .next()
            .expect("the input of a shared node");
        Ok(Self {
            id: self.id,
            input,
            schema: Arc::clone(&self.schema),
        })
    }
}

// ---------------------------------------------------------------------------------------
// The rule that inlines cheap inputs
// ---------------------------------------------------------------------------------------

/// Replaces a shared node with its input when the input is cheap to compute again.
#[derive(Debug, Default)]
pub struct InlineShared;

impl OptimizerRule for InlineShared {
    fn name(&self) -> &str {
        "inline_shared"
    }

    fn apply_order(&self) -> Option<ApplyOrder> {
        Some(ApplyOrder::BottomUp)
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        _config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::Extension(extension) = &plan else {
            return Ok(Transformed::no(plan));
        };
        let Some(node) = extension.node.as_any().downcast_ref::<SharedNode>() else {
            return Ok(Transformed::no(plan));
        };
        match cheap(&node.input)? {
            true => Ok(Transformed::yes(node.input.clone())),
            false => Ok(Transformed::no(plan)),
        }
    }
}

/// Whether `plan` is cheap to compute again and computes nothing volatile: a `VALUES` list, an
/// empty relation, or a scan of an in-memory table, under projections, filters, and aliases.
fn cheap(plan: &LogicalPlan) -> Result<bool> {
    if plan.expressions().iter().any(Expr::is_volatile) {
        return Ok(false);
    }
    Ok(match plan {
        LogicalPlan::Values(_) | LogicalPlan::EmptyRelation(_) => true,
        LogicalPlan::TableScan(scan) => {
            !scan.filters.iter().any(Expr::is_volatile)
                && source_as_provider(&scan.source).is_ok_and(|provider| {
                    let provider: &dyn Any = provider.as_ref();
                    provider.is::<MemTable>()
                })
        }
        LogicalPlan::Projection(_) | LogicalPlan::Filter(_) | LogicalPlan::SubqueryAlias(_) => {
            cheap(plan.inputs()[0])?
        }
        _ => false,
    })
}

// ---------------------------------------------------------------------------------------
// The rule that prunes the columns of shared inputs
// ---------------------------------------------------------------------------------------

/// Narrows the input of each shared node to the union of the columns that its references' readers
/// read. DataFusion's pushdown of projections stops at a shared node, since each reference sees
/// only its own reader, so without this rule an input computes and buffers every column. The rule
/// reads the readers that the pushdown leaves directly above each reference: a projection or an
/// aggregate reads the columns of its expressions, and any other reader, or a reference at the
/// root of a plan or a subquery, reads every column, so its node keeps its input. Each pruned
/// input is a projection of the original, so every reference of a node keeps one input, and the
/// next pushdown carries the projection into the input.
///
/// Place it after DataFusion's `optimize_projections`.
#[derive(Debug, Default)]
pub struct PruneShared;

impl OptimizerRule for PruneShared {
    fn name(&self) -> &str {
        "prune_shared"
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        _config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let kept = kept_columns(&plan)?;
        if kept.is_empty() {
            return Ok(Transformed::no(plan));
        }
        plan.transform_up_with_subqueries(|plan| {
            let Some(node) = as_shared(&plan) else {
                return Ok(Transformed::no(plan));
            };
            let Some(columns) = kept.get(&node.id) else {
                return Ok(Transformed::no(plan));
            };
            let schema = node.input.schema();
            let exprs = (columns.iter())
                .map(|index| Expr::Column(Column::from(schema.qualified_field(*index))))
                .collect();
            let input = Projection::try_new(exprs, Arc::new(node.input.clone()))?;
            let input = LogicalPlan::Projection(input);
            Ok(Transformed::yes(LogicalPlan::Extension(Extension {
                node: Arc::new(SharedNode {
                    id: node.id,
                    schema: Arc::clone(input.schema()),
                    input,
                }),
            })))
        })
    }
}

/// The shared node that `plan` is, if it is one.
fn as_shared(plan: &LogicalPlan) -> Option<&SharedNode> {
    let LogicalPlan::Extension(extension) = plan else {
        return None;
    };
    extension.node.as_any().downcast_ref::<SharedNode>()
}

/// The columns, by position, that the readers of each shared node of `plan` read, for each node
/// whose readers all read known columns and leave some unread.
fn kept_columns(plan: &LogicalPlan) -> Result<HashMap<u64, BTreeSet<usize>>> {
    // For each node: its references, its width, and the references whose reader reads known
    // columns, with those columns.
    let mut nodes: HashMap<u64, (usize, usize, usize, BTreeSet<usize>)> = HashMap::new();
    plan.apply_with_subqueries(|parent| {
        if let Some(node) = as_shared(parent) {
            let width = node.schema.fields().len();
            nodes
                .entry(node.id)
                .or_insert((0, width, 0, BTreeSet::new()))
                .0 += 1;
        }
        for child in parent.inputs() {
            let Some(node) = as_shared(child) else {
                continue;
            };
            if let Some(columns) = columns_read(parent, child) {
                let width = node.schema.fields().len();
                let entry = nodes
                    .entry(node.id)
                    .or_insert((0, width, 0, BTreeSet::new()));
                entry.2 += 1;
                entry.3.extend(columns);
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok((nodes.into_iter())
        .filter(|(_, (references, width, known, columns))| {
            known == references && columns.len() < *width
        })
        .map(|(id, (.., mut columns))| {
            // An input of no columns would lose its rows, which a count reads.
            if columns.is_empty() {
                columns.insert(0);
            }
            (id, columns)
        })
        .collect())
}

/// The columns of `child`, by position, that its reader `parent` reads, or `None` when the
/// reader can read every column, such as a filter, which passes them all up.
fn columns_read(parent: &LogicalPlan, child: &LogicalPlan) -> Option<BTreeSet<usize>> {
    let exprs: Vec<&Expr> = match parent {
        LogicalPlan::Projection(projection) => projection.expr.iter().collect(),
        LogicalPlan::Aggregate(aggregate) => (aggregate.group_expr.iter())
            .chain(&aggregate.aggr_expr)
            .collect(),
        _ => return None,
    };
    let schema = child.schema();
    let mut columns = BTreeSet::new();
    for expr in exprs {
        for column in expr.column_refs() {
            columns.insert(schema.maybe_index_of_column(column)?);
        }
    }
    Some(columns)
}

// ---------------------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------------------

/// The batches of one output partition of a shared input: the stream of the partition, which
/// the first reader starts, and the batches that some reader has not read yet.
struct Partition {
    stream: Option<SendableRecordBatchStream>,
    done: bool,
    /// The batches from the index `first` on, with the number of readers that have read each.
    batches: VecDeque<(RecordBatch, usize)>,
    first: usize,
    reservation: Option<MemoryReservation>,
}

/// The input of the references to one shared node, optimized once, and its buffered batches.
struct SharedState {
    id: u64,
    input: Arc<dyn ExecutionPlan>,
    /// The number of readers, each a [`SharedExec`].
    readers: Mutex<usize>,
    partitions: Vec<AsyncMutex<Partition>>,
}

impl SharedState {
    fn new(id: u64, input: Arc<dyn ExecutionPlan>) -> Self {
        let count = input.properties().partitioning.partition_count();
        let partitions = (0..count)
            .map(|_| {
                AsyncMutex::new(Partition {
                    stream: None,
                    done: false,
                    batches: VecDeque::new(),
                    first: 0,
                    reservation: None,
                })
            })
            .collect();
        Self {
            id,
            input,
            readers: Mutex::new(0),
            partitions,
        }
    }

    fn readers(&self) -> usize {
        *self.readers.lock().expect("the readers of a shared input")
    }

    /// The batch at `index` of `partition`, for a reader that has read the batches before it,
    /// or None after the last batch.
    async fn batch(
        &self,
        partition: usize,
        index: usize,
        context: &Arc<TaskContext>,
    ) -> Result<Option<RecordBatch>> {
        let readers = self.readers();
        let mut p = self.partitions[partition].lock().await;
        if index >= p.first + p.batches.len() {
            if p.done {
                return Ok(None);
            }
            if p.stream.is_none() {
                p.stream = Some(self.input.execute(partition, Arc::clone(context))?);
                p.reservation = Some(
                    MemoryConsumer::new(format!("SharedExec[{}][{partition}]", self.id))
                        .register(context.memory_pool()),
                );
            }
            let next = p.stream.as_mut().expect("a started stream").next().await;
            match next {
                Some(batch) => {
                    let batch = batch?;
                    let size = batch.get_array_memory_size();
                    (p.reservation.as_mut())
                        .expect("a reservation")
                        .try_grow(size)?;
                    p.batches.push_back((batch, 0));
                }
                None => {
                    p.done = true;
                    p.stream = None;
                    return Ok(None);
                }
            }
        }
        let at = index - p.first;
        let (batch, read) = &mut p.batches[at];
        let batch = batch.clone();
        *read += 1;
        // Frees the batches at the front that every reader has read.
        while p.batches.front().is_some_and(|(_, read)| *read >= readers) {
            let (freed, _) = p.batches.pop_front().expect("a batch at the front");
            p.first += 1;
            if let Some(reservation) = p.reservation.as_mut() {
                reservation.shrink(freed.get_array_memory_size());
            }
        }
        Ok(Some(batch))
    }
}

/// A reader of a shared input. Every reader of one shared node reads the rows of one run of
/// its input.
pub struct SharedExec {
    state: Arc<SharedState>,
    properties: Arc<PlanProperties>,
}

impl SharedExec {
    fn new(state: Arc<SharedState>) -> Self {
        *state.readers.lock().expect("the readers of a shared input") += 1;
        let properties = Arc::clone(state.input.properties());
        Self { state, properties }
    }
}

impl fmt::Debug for SharedExec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SharedExec {{ id: {} }}", self.state.id)
    }
}

impl DisplayAs for SharedExec {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "SharedExec: id={}, readers={}",
            self.state.id,
            self.state.readers()
        )
    }
}

impl ExecutionPlan for SharedExec {
    fn name(&self) -> &str {
        "SharedExec"
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }
    // The input is a part of the shared state, which the planner optimized once, so the
    // physical optimizer sees a leaf, and leaves the input the same for every reader.
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }
    fn apply_expressions(
        &self,
        _f: &mut dyn FnMut(&Arc<dyn PhysicalExpr>) -> Result<TreeNodeRecursion>,
    ) -> Result<TreeNodeRecursion> {
        Ok(TreeNodeRecursion::Continue)
    }
    fn with_new_children(
        self: Arc<Self>,
        _children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        Ok(self)
    }
    fn replace_children(
        self: Arc<Self>,
        _children: Vec<Arc<dyn ExecutionPlan>>,
        _options: ReplaceChildrenOptions,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        Ok(self)
    }
    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        let state = Arc::clone(&self.state);
        let stream = futures::stream::unfold(Some(0usize), move |index| {
            let state = Arc::clone(&state);
            let context = Arc::clone(&context);
            async move {
                let index = index?;
                match state.batch(partition, index, &context).await {
                    Ok(Some(batch)) => Some((Ok(batch), Some(index + 1))),
                    Ok(None) => None,
                    Err(error) => Some((Err(error), None)),
                }
            }
        });
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            self.state.input.schema(),
            stream,
        )))
    }
}

// ---------------------------------------------------------------------------------------
// Physical planning
// ---------------------------------------------------------------------------------------

/// Plans the references to each [`SharedNode`] as [`SharedExec`]s of one shared state. A planner
/// serves the planning of one query, as a query planner makes one for each plan.
#[derive(Default)]
pub struct SharedExtensionPlanner {
    states: Mutex<HashMap<u64, Arc<SharedState>>>,
}

impl fmt::Debug for SharedExtensionPlanner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedExtensionPlanner")
    }
}

#[async_trait]
impl ExtensionPlanner for SharedExtensionPlanner {
    async fn plan_extension(
        &self,
        _planner: &dyn PhysicalPlanner,
        node: &dyn UserDefinedLogicalNode,
        _logical_inputs: &[&LogicalPlan],
        physical_inputs: &[Arc<dyn ExecutionPlan>],
        session: &dyn Session,
        _ctx: &PhysicalPlanningContext,
    ) -> Result<Option<Arc<dyn ExecutionPlan>>> {
        let Some(node) = node.as_any().downcast_ref::<SharedNode>() else {
            return Ok(None);
        };
        let known = self
            .states
            .lock()
            .expect("the states of shared inputs")
            .get(&node.id)
            .cloned();
        let state = match known {
            // A later reference reads the state of the first, and its own copy of the input,
            // which the physical planner planned, is never run.
            Some(state) => state,
            None => {
                let input = DefaultPhysicalPlanner::default().optimize_physical_plan(
                    Arc::clone(&physical_inputs[0]),
                    session,
                    |_, _| {},
                )?;
                let state = Arc::new(SharedState::new(node.id, input));
                self.states
                    .lock()
                    .expect("the states of shared inputs")
                    .insert(node.id, Arc::clone(&state));
                state
            }
        };
        Ok(Some(Arc::new(SharedExec::new(state))))
    }
}
