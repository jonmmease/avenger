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
//! [`SharedExtensionPlanner`] plans the references to one node as [`SharedExec`]s of one shared
//! state, which holds the input, optimized once. The first reader of an output partition to run
//! drives the input partition, and the state buffers its batches, under a memory reservation, for
//! the other readers. A batch is freed when every reader has read it.

use std::any::Any;
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use datafusion::arrow::array::RecordBatch;
use datafusion::catalog::{MemTable, Session};
use datafusion::common::tree_node::{Transformed, TreeNodeRecursion};
use datafusion::common::{DFSchemaRef, Result};
use datafusion::datasource::source_as_provider;
use datafusion::execution::memory_pool::{MemoryConsumer, MemoryReservation};
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::physical_planning_context::PhysicalPlanningContext;
use datafusion::logical_expr::{
    Expr, Extension, LogicalPlan, UserDefinedLogicalNode, UserDefinedLogicalNodeCore,
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

/// An input that its readers share. The references to one node have one `id`.
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
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = NEXT.fetch_add(1, AtomicOrdering::Relaxed);
    let schema = Arc::clone(input.schema());
    LogicalPlan::Extension(Extension {
        node: Arc::new(SharedNode { id, input, schema }),
    })
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
