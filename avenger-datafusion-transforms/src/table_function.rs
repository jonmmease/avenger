//! Table functions over an input relation, such as `impute`: a [`TableTransform`] of the rows
//! of each partition of its input, which [`TableFunctionNode`] holds in a logical plan and
//! [`TableFunctionExec`] runs. The exec reads the input of its task into memory, then transforms
//! each partition, the rows of one value of the `partition_by` columns, with the transform's
//! [`PartitionTransform`]; the input is hash partitioned on those columns, so partitions run in
//! parallel. Without them, the input is one partition.
//!
//! A table function plans its call from its input's plan and its arguments, as
//! [`crate::impute::plan`] does, and reports an error of an argument as an [`ArgError`] at the
//! argument, which the caller places at its text. DataFusion's own table functions take
//! constants and return a table, so they cannot read a relation or bind arguments to its
//! columns.

use std::any::Any;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{ArrayRef, RecordBatch, RecordBatchOptions, UInt32Array};
use datafusion::arrow::compute::{concat_batches, take_record_batch};
use datafusion::arrow::datatypes::{DataType, Schema, SchemaRef};
use datafusion::arrow::row::{RowConverter, SortField};
use datafusion::catalog::Session;
use datafusion::common::tree_node::TreeNodeRecursion;
use datafusion::common::{Column, DFSchema, DFSchemaRef, DataFusionError, Result, ScalarValue};
use datafusion::execution::context::QueryPlanner;
use datafusion::execution::memory_pool::MemoryConsumer;
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::execution_props::ExecutionProps;
use datafusion::logical_expr::physical_planning_context::PhysicalPlanningContext;
use datafusion::logical_expr::{
    ColumnarValue, Expr, LogicalPlan, LogicalPlanBuilder, UserDefinedLogicalNode,
    UserDefinedLogicalNodeCore,
};
use datafusion::physical_expr::expressions::Column as PhysicalColumn;
use datafusion::physical_expr::{EquivalenceProperties, PhysicalExpr, create_physical_expr};
use datafusion::physical_plan::execution_plan::EmissionType;
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::{
    ChildrenPropertiesMode, DisplayAs, DisplayFormatType, Distribution, ExecutionPlan,
    InputDistributionRequirements, Partitioning, PlanProperties, ReplaceChildrenOptions,
};
use datafusion::physical_planner::{DefaultPhysicalPlanner, ExtensionPlanner, PhysicalPlanner};
use futures::StreamExt;

// ---------------------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------------------

/// Where an error of a call of a table function is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgAt {
    /// The call, or the function's name.
    Call,
    /// The positional argument at this index, after the input.
    Positional(usize),
    /// The element at this index of the list of the positional argument at the first index.
    Element(usize, usize),
    /// The named argument at this index, in the order of the call.
    Named(usize),
    /// The element at this index of the list of the named argument at the first index.
    NamedElement(usize, usize),
}

/// An error of a call of a table function, at one of its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgError {
    pub at: ArgAt,
    pub message: String,
}

impl ArgError {
    pub fn new(at: ArgAt, message: impl Into<String>) -> Self {
        Self {
            at,
            message: message.into(),
        }
    }
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<DataFusionError> for ArgError {
    fn from(error: DataFusionError) -> Self {
        ArgError::new(ArgAt::Call, error.strip_backtrace())
    }
}

/// The value of a constant expression, after type coercion (`1 + 0.5`).
pub fn constant(expr: &Expr) -> Result<ScalarValue> {
    let schema = DFSchema::empty();
    let simplifier = datafusion::optimizer::simplify_expressions::ExprSimplifier::new(
        datafusion::logical_expr::simplify::SimplifyContext::default(),
    );
    let expr = simplifier.coerce(expr.clone(), &schema)?;
    let physical: Arc<dyn PhysicalExpr> = create_physical_expr(
        &expr,
        &schema,
        &ExecutionProps::new(),
        &PhysicalPlanningContext::default(),
    )?;
    let batch = RecordBatch::try_new_with_options(
        Arc::new(Schema::empty()),
        vec![],
        &RecordBatchOptions::new().with_row_count(Some(1)),
    )?;
    match physical.evaluate(&batch)? {
        ColumnarValue::Scalar(s) => Ok(s),
        ColumnarValue::Array(a) => ScalarValue::try_from_array(&a, 0),
    }
}

/// A constant cast to a type, or None if the cast fails, makes a NULL of a value, or loses
/// part of a number (`0.5` as a BIGINT).
pub fn cast_exact(v: &ScalarValue, to: &DataType) -> Option<ScalarValue> {
    let cast = v.cast_to(to).ok().filter(|c| c.is_null() == v.is_null())?;
    let from = v.data_type();
    if !v.is_null()
        && from.is_numeric()
        && to.is_numeric()
        && cast.cast_to(&from).ok().as_ref() != Some(v)
    {
        return None;
    }
    Some(cast)
}

// ---------------------------------------------------------------------------------------
// Functions and transforms
// ---------------------------------------------------------------------------------------

/// The equality and the hash of a transform, for the equality of logical plans.
pub trait DynTransform {
    fn as_any(&self) -> &dyn Any;
    fn dyn_eq(&self, other: &dyn Any) -> bool;
    fn dyn_hash(&self, state: &mut dyn Hasher);
}

impl<T: Any + PartialEq + Hash> DynTransform for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn dyn_eq(&self, other: &dyn Any) -> bool {
        other.downcast_ref::<T>() == Some(self)
    }
    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        self.hash(&mut state);
    }
}

/// What a table function does to its input, in a logical plan.
pub trait TableTransform: DynTransform + fmt::Debug + Send + Sync {
    /// The name of the logical node, such as `Impute`.
    fn name(&self) -> &'static str;
    /// The columns of the input that the transform reads.
    fn columns(&self) -> Vec<Column>;
    /// The transform with `columns` in place of its columns, in the order of [`Self::columns`].
    fn with_columns(&self, columns: Vec<Column>) -> Result<Arc<dyn TableTransform>>;
    /// The schema of the result of `input`, which `partition_by` partitions.
    fn schema(&self, input: &DFSchema, partition_by: &[Column]) -> Result<DFSchemaRef>;
    /// The transform of each partition, with the columns as indices of `input`.
    fn partition_transform(&self, input: &DFSchema) -> Result<Arc<dyn PartitionTransform>>;
    /// The transform in `EXPLAIN`, after the node's name.
    fn fmt_for_explain(&self, f: &mut fmt::Formatter) -> fmt::Result;
}

/// The transform of the rows of one partition, at execution.
pub trait PartitionTransform: fmt::Debug + Send + Sync {
    /// The result of the rows of a partition. `partitions` are the indices of the partition
    /// columns, constant in the rows; `schema` is the schema of the result.
    fn transform(
        &self,
        input: &RecordBatch,
        partitions: &[usize],
        schema: SchemaRef,
    ) -> Result<RecordBatch>;
}

// ---------------------------------------------------------------------------------------
// The logical node
// ---------------------------------------------------------------------------------------

/// A call of a table function in a logical plan.
#[derive(Debug, Clone)]
pub struct TableFunctionNode {
    pub input: LogicalPlan,
    pub partition_by: Vec<Column>,
    pub transform: Arc<dyn TableTransform>,
    pub schema: DFSchemaRef,
}

impl TableFunctionNode {
    pub fn try_new(
        input: LogicalPlan,
        partition_by: Vec<Column>,
        transform: Arc<dyn TableTransform>,
    ) -> Result<Self> {
        let schema = transform.schema(input.schema(), &partition_by)?;
        Ok(Self {
            input,
            partition_by,
            transform,
            schema,
        })
    }
}

impl PartialEq for TableFunctionNode {
    fn eq(&self, other: &Self) -> bool {
        self.input == other.input
            && self.partition_by == other.partition_by
            && self.transform.dyn_eq(other.transform.as_any())
    }
}

impl Eq for TableFunctionNode {}

impl Hash for TableFunctionNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.input.hash(state);
        self.partition_by.hash(state);
        self.transform.dyn_hash(state);
    }
}

impl PartialOrd for TableFunctionNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (self == other).then_some(Ordering::Equal)
    }
}

impl UserDefinedLogicalNodeCore for TableFunctionNode {
    fn name(&self) -> &str {
        self.transform.name()
    }
    fn inputs(&self) -> Vec<&LogicalPlan> {
        vec![&self.input]
    }
    fn schema(&self) -> &DFSchemaRef {
        &self.schema
    }
    fn expressions(&self) -> Vec<Expr> {
        self.transform
            .columns()
            .into_iter()
            .chain(self.partition_by.iter().cloned())
            .map(Expr::Column)
            .collect()
    }
    fn fmt_for_explain(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: ", self.transform.name())?;
        self.transform.fmt_for_explain(f)?;
        if !self.partition_by.is_empty() {
            let parts: Vec<_> = self.partition_by.iter().map(|c| c.to_string()).collect();
            write!(f, ", partition_by=[{}]", parts.join(", "))?;
        }
        Ok(())
    }
    fn with_exprs_and_inputs(&self, exprs: Vec<Expr>, inputs: Vec<LogicalPlan>) -> Result<Self> {
        let mut columns = exprs
            .into_iter()
            .map(|e| match e {
                Expr::Column(c) => Ok(c),
                e => datafusion::common::internal_err!(
                    "{} expects columns, not {e}",
                    self.transform.name()
                ),
            })
            .collect::<Result<Vec<_>>>()?;
        let partition_by = columns.split_off(columns.len() - self.partition_by.len());
        let input = inputs.into_iter().next().expect("one input");
        // A rebuild over an input with more columns, such as DataFusion's pushdown of a struct
        // field access (`key['month']`) into the input: added rows would be NULL in those
        // columns, so the node keeps the columns it had, and the pushdown stops above it.
        let old = self.input.schema().columns();
        let input = if input.schema().fields().len() > old.len()
            && old.iter().all(|c| input.schema().has_column(c))
        {
            LogicalPlanBuilder::from(input)
                .project(old.into_iter().map(Expr::Column))?
                .build()?
        } else {
            input
        };
        Self::try_new(input, partition_by, self.transform.with_columns(columns)?)
    }
}

// ---------------------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------------------

/// Runs a table function: reads the input of its task into memory, then transforms each
/// partition, in the order in which the partitions first occur.
#[derive(Debug)]
pub struct TableFunctionExec {
    input: Arc<dyn ExecutionPlan>,
    transform: Arc<dyn PartitionTransform>,
    /// The indices of the partition columns.
    partitions: Vec<usize>,
    schema: SchemaRef,
    properties: Arc<PlanProperties>,
}

impl TableFunctionExec {
    pub fn new(
        input: Arc<dyn ExecutionPlan>,
        transform: Arc<dyn PartitionTransform>,
        partitions: Vec<usize>,
        schema: SchemaRef,
    ) -> Self {
        // With partitions, each task transforms the partitions of its input, which keep their
        // hash partitioning.
        let partitioning = if partitions.is_empty() {
            Partitioning::UnknownPartitioning(1)
        } else {
            input.properties().output_partitioning().clone()
        };
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(schema.clone()),
            partitioning,
            EmissionType::Final,
            input.properties().boundedness,
        ));
        Self {
            input,
            transform,
            partitions,
            schema,
            properties,
        }
    }
}

impl DisplayAs for TableFunctionExec {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "TableFunctionExec: {:?}", self.transform)?;
        if !self.partitions.is_empty() {
            write!(f, ", partitions={:?}", self.partitions)?;
        }
        Ok(())
    }
}

impl ExecutionPlan for TableFunctionExec {
    fn name(&self) -> &str {
        "TableFunctionExec"
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }
    fn input_distribution_requirements(&self) -> InputDistributionRequirements {
        let distribution = if self.partitions.is_empty() {
            Distribution::SinglePartition
        } else {
            let schema = self.input.schema();
            Distribution::KeyPartitioned(
                self.partitions
                    .iter()
                    .map(|&i| {
                        Arc::new(PhysicalColumn::new(schema.field(i).name(), i))
                            as Arc<dyn PhysicalExpr>
                    })
                    .collect(),
            )
        };
        InputDistributionRequirements::new(vec![distribution])
    }
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![&self.input]
    }
    fn apply_expressions(
        &self,
        _f: &mut dyn FnMut(&Arc<dyn PhysicalExpr>) -> Result<TreeNodeRecursion>,
    ) -> Result<TreeNodeRecursion> {
        Ok(TreeNodeRecursion::Continue)
    }
    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        self.replace_children(
            children,
            ReplaceChildrenOptions::new(ChildrenPropertiesMode::Recompute),
        )
    }
    fn replace_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
        _options: ReplaceChildrenOptions,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        Ok(Arc::new(Self::new(
            children[0].clone(),
            self.transform.clone(),
            self.partitions.clone(),
            self.schema.clone(),
        )))
    }
    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        let batch_size = context.session_config().batch_size().max(1);
        let reservation = MemoryConsumer::new(format!("TableFunctionExec[{partition}]"))
            .register(context.memory_pool());
        let mut input = self.input.execute(partition, context)?;
        let input_schema = self.input.schema();
        let schema = self.schema.clone();
        let transform = self.transform.clone();
        let partitions = self.partitions.clone();
        let out = schema.clone();
        let stream = futures::stream::once(async move {
            let mut batches = Vec::new();
            while let Some(batch) = input.next().await {
                let batch = batch?;
                reservation.try_grow(batch.get_array_memory_size())?;
                batches.push(batch);
            }
            let input = concat_batches(&input_schema, &batches)?;
            drop(batches);
            let result = transform_partitions(&input, transform.as_ref(), &partitions, out);
            drop(reservation);
            result
        })
        .flat_map(move |result| {
            let parts: Vec<Result<RecordBatch>> = match result {
                Ok(b) => (0..b.num_rows())
                    .step_by(batch_size)
                    .map(|i| Ok(b.slice(i, batch_size.min(b.num_rows() - i))))
                    .collect(),
                Err(e) => vec![Err(e)],
            };
            futures::stream::iter(parts)
        })
        .boxed();
        Ok(Box::pin(RecordBatchStreamAdapter::new(schema, stream)))
    }
}

/// Transforms each partition of the rows of one task: the rows of one value of the partition
/// columns, in the order in which the values first occur.
fn transform_partitions(
    input: &RecordBatch,
    transform: &dyn PartitionTransform,
    partitions: &[usize],
    schema: SchemaRef,
) -> Result<RecordBatch> {
    if partitions.is_empty() {
        return transform.transform(input, partitions, schema);
    }
    let columns: Vec<ArrayRef> = partitions
        .iter()
        .map(|&i| input.column(i).clone())
        .collect();
    let conv = RowConverter::new(
        columns
            .iter()
            .map(|c| SortField::new(c.data_type().clone()))
            .collect(),
    )?;
    let mut index: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut rows: Vec<Vec<u32>> = Vec::new();
    for (i, b) in row_bytes(&conv, &columns)?.into_iter().enumerate() {
        let p = *index.entry(b).or_insert_with(|| {
            rows.push(Vec::new());
            rows.len() - 1
        });
        rows[p].push(i as u32);
    }
    let mut outputs = Vec::with_capacity(rows.len());
    for r in rows {
        let part = take_record_batch(input, &UInt32Array::from(r))?;
        outputs.push(transform.transform(&part, partitions, schema.clone())?);
    }
    Ok(concat_batches(&schema, &outputs)?)
}

/// Owned row bytes, to hash and compare values of any type.
pub(crate) fn row_bytes(conv: &RowConverter, columns: &[ArrayRef]) -> Result<Vec<Vec<u8>>> {
    let rows = conv.convert_columns(columns)?;
    Ok(rows.iter().map(|r| r.as_ref().to_vec()).collect())
}

// ---------------------------------------------------------------------------------------
// Physical planning
// ---------------------------------------------------------------------------------------

/// Plans [`TableFunctionNode`] as [`TableFunctionExec`].
#[derive(Debug)]
pub struct TableFunctionExtensionPlanner;

#[async_trait]
impl ExtensionPlanner for TableFunctionExtensionPlanner {
    async fn plan_extension(
        &self,
        _planner: &dyn PhysicalPlanner,
        node: &dyn UserDefinedLogicalNode,
        logical_inputs: &[&LogicalPlan],
        physical_inputs: &[Arc<dyn ExecutionPlan>],
        _session: &dyn Session,
        _ctx: &PhysicalPlanningContext,
    ) -> Result<Option<Arc<dyn ExecutionPlan>>> {
        let Some(n) = node.as_any().downcast_ref::<TableFunctionNode>() else {
            return Ok(None);
        };
        let schema = logical_inputs[0].schema();
        let partitions = n
            .partition_by
            .iter()
            .map(|c| schema.index_of_column(c))
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Arc::new(TableFunctionExec::new(
            physical_inputs[0].clone(),
            n.transform.partition_transform(schema)?,
            partitions,
            Arc::new(n.schema.as_arrow().clone()),
        ))))
    }
}

/// DataFusion's default query planner, with [`TableFunctionExtensionPlanner`].
#[derive(Debug)]
pub struct TransformsQueryPlanner;

#[async_trait]
impl QueryPlanner for TransformsQueryPlanner {
    async fn create_physical_plan(
        &self,
        logical_plan: &LogicalPlan,
        session: &dyn Session,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        DefaultPhysicalPlanner::with_extension_planners(vec![Arc::new(
            TableFunctionExtensionPlanner,
        )])
        .create_physical_plan(logical_plan, session)
        .await
    }
}
