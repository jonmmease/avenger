//! Table functions over an input relation, such as `impute`:
//!
//! ```sql
//! SELECT * FROM name(input, positional, ..., named => value, ..., partition_by => [p, ...])
//! ```
//!
//! The input is a subquery, the name of a CTE of the statement, or the name of a table. The
//! positional arguments follow it, then the named arguments. Every function takes
//! `partition_by`, columns whose values the function transforms separately: the input is hash
//! partitioned on them, so partitions run in parallel. Without it, the input is one partition.
//!
//! A [`TableFunction`] declares its arguments and plans a call into a [`TableTransform`].
//! [`TableFunctionPlanner`], a relation planner, finds the calls, plans their inputs, splits
//! their arguments, binds `partition_by`, and reports errors at the arguments' places.
//! [`TableFunctionNode`] holds a transform in a logical plan, and [`TableFunctionExec`] runs
//! it: it reads the input of its task into memory, then transforms each partition with the
//! transform's [`PartitionTransform`].
//!
//! DataFusion's own table functions take constants and return a table, so they cannot read a
//! relation or bind arguments to its columns.

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
use datafusion::catalog::{CatalogProviderList, Session};
use datafusion::common::tree_node::TreeNodeRecursion;
use datafusion::common::{
    Column, DFSchema, DFSchemaRef, DataFusionError, Diagnostic, Result, ScalarValue, Span,
};
use datafusion::datasource::provider_as_source;
use datafusion::execution::context::QueryPlanner;
use datafusion::execution::memory_pool::MemoryConsumer;
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::execution_props::ExecutionProps;
use datafusion::logical_expr::physical_planning_context::PhysicalPlanningContext;
use datafusion::logical_expr::planner::{
    PlannedRelation, RelationPlanner, RelationPlannerContext, RelationPlanning,
};
use datafusion::logical_expr::{
    ColumnarValue, Expr, Extension, LogicalPlan, LogicalPlanBuilder, UserDefinedLogicalNode,
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
use datafusion::sql::sqlparser::ast::{
    Expr as SqlExpr, FunctionArg, FunctionArgExpr, Ident, ObjectName, Spanned, TableFactor,
};
use datafusion::sql::sqlparser::tokenizer::Span as SqlSpan;
use futures::{FutureExt, StreamExt};

// ---------------------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------------------

/// An error at the place of an argument.
pub fn err_at(span: SqlSpan, message: impl Into<String>) -> DataFusionError {
    let message = message.into();
    DataFusionError::Diagnostic(
        Box::new(Diagnostic::new_error(
            message.clone(),
            Span::try_from_sqlparser_span(span),
        )),
        Box::new(DataFusionError::Plan(message)),
    )
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

/// A call of a table function: its planned input, its arguments, and the binding of the
/// arguments, whose errors point at the arguments.
pub struct Call<'a> {
    pub ctx: &'a mut dyn RelationPlannerContext,
    /// The name of the function.
    pub name: &'static str,
    /// The place of the function's name.
    pub span: SqlSpan,
    pub input: LogicalPlan,
    /// The positional arguments after the input, as many as the function declares.
    pub positional: Vec<SqlExpr>,
    /// The named arguments other than `partition_by`, in order, by their names in lowercase.
    /// Each is one of the function's names and occurs once.
    pub named: Vec<(String, SqlExpr)>,
    /// The columns of `partition_by`.
    pub partition_by: Vec<Column>,
}

impl Call<'_> {
    pub fn schema(&self) -> DFSchemaRef {
        self.input.schema().clone()
    }

    /// A column of the input.
    pub fn column(&mut self, what: &str, e: SqlExpr) -> Result<Column> {
        column(self.ctx, self.name, what, e, self.input.schema())
    }

    /// The columns of a list, or one column without brackets, with their places.
    pub fn columns(&mut self, what: &str, e: SqlExpr) -> Result<Vec<(Column, SqlSpan)>> {
        let exprs = match e {
            SqlExpr::Array(a) => a.elem,
            e => vec![e],
        };
        exprs
            .into_iter()
            .map(|e| {
                let span = e.span();
                Ok((self.column(what, e)?, span))
            })
            .collect()
    }

    /// The value of a constant argument.
    pub fn constant(&mut self, what: &str, e: SqlExpr) -> Result<ScalarValue> {
        let span = e.span();
        let not_constant = |e: DataFusionError| {
            err_at(
                span,
                format!("{what} must be a constant: {}", e.strip_backtrace()),
            )
        };
        let expr = self
            .ctx
            .sql_to_expr(e, &DFSchema::empty())
            .map_err(not_constant)?;
        constant(&expr).map_err(not_constant)
    }

    /// The constants of a list, each cast exactly to `data_type`, without repeats. `what`
    /// names the list, such as "`keys`", and `of` the type, such as "the key".
    pub fn constant_list(
        &mut self,
        what: &str,
        e: SqlExpr,
        data_type: &DataType,
        of: &str,
    ) -> Result<Vec<ScalarValue>> {
        let span = e.span();
        let SqlExpr::Array(array) = e else {
            return Err(err_at(
                span,
                format!("{what} must be a list of constants, such as [1, 2]"),
            ));
        };
        let mut out: Vec<ScalarValue> = Vec::new();
        for elem in array.elem {
            let span = elem.span();
            let v = self.constant(&format!("a value of {what}"), elem)?;
            if v.is_null() {
                return Err(err_at(span, format!("a value of {what} cannot be NULL")));
            }
            let v = cast_exact(&v, data_type).ok_or_else(|| {
                let shown = if v.try_as_str().is_some() {
                    format!("'{v}'")
                } else {
                    v.to_string()
                };
                err_at(
                    span,
                    format!("the value {shown} of {what} does not coerce to the type of {of}, {data_type}"),
                )
            })?;
            if !out.contains(&v) {
                out.push(v);
            }
        }
        Ok(out)
    }
}

fn column(
    ctx: &mut dyn RelationPlannerContext,
    function: &str,
    what: &str,
    e: SqlExpr,
    schema: &DFSchema,
) -> Result<Column> {
    let span = e.span();
    match ctx.sql_to_expr(e, schema) {
        Ok(Expr::Column(c)) => Ok(c),
        Ok(other) => Err(err_at(
            span,
            format!("the {what} of {function} must be a column of its input, not {other}"),
        )),
        Err(e) => Err(err_at(
            span,
            format!(
                "the {what} of {function} must be a column of its input: {}",
                e.strip_backtrace().lines().next().unwrap_or("")
            ),
        )),
    }
}

// ---------------------------------------------------------------------------------------
// Functions and transforms
// ---------------------------------------------------------------------------------------

/// A table function: its arguments, and the transform of a call.
pub trait TableFunction: fmt::Debug + Send + Sync {
    /// The name of the function in SQL.
    fn name(&self) -> &'static str;
    /// The positional arguments after the input, by what they are, such as "the key".
    fn positional(&self) -> &'static [&'static str];
    /// The names of the named arguments other than `partition_by`.
    fn named(&self) -> &'static [&'static str];
    /// The transform of a call. The columns of `partition_by` are not columns of the
    /// transform.
    fn plan(&self, call: Call<'_>) -> Result<Arc<dyn TableTransform>>;
}

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
// The relation planner
// ---------------------------------------------------------------------------------------

/// Plans the calls of table functions in FROM clauses.
#[derive(Debug)]
pub struct TableFunctionPlanner {
    functions: Vec<Arc<dyn TableFunction>>,
    /// The session's catalogs, for inputs that name a table.
    catalogs: Arc<dyn CatalogProviderList>,
}

impl TableFunctionPlanner {
    pub fn new(
        functions: Vec<Arc<dyn TableFunction>>,
        catalogs: Arc<dyn CatalogProviderList>,
    ) -> Self {
        Self {
            functions,
            catalogs,
        }
    }

    /// The input: a subquery, a CTE of the statement, or a table.
    fn input(
        &self,
        ctx: &mut dyn RelationPlannerContext,
        function: &str,
        e: SqlExpr,
    ) -> Result<LogicalPlan> {
        let span = e.span();
        let name = match e {
            SqlExpr::Subquery(query) => {
                return ctx.plan(TableFactor::Derived {
                    lateral: false,
                    subquery: query,
                    alias: None,
                    sample: None,
                });
            }
            SqlExpr::Identifier(id) => ObjectName::from(vec![id]),
            SqlExpr::CompoundIdentifier(ids) => ObjectName::from(ids),
            _ => {
                return Err(err_at(
                    span,
                    format!("the input of {function} must be a table name or a subquery"),
                ));
            }
        };
        // As `FROM name`: a CTE of the statement, or a table that the statement reads.
        let table = TableFactor::Table {
            name: name.clone(),
            alias: None,
            args: None,
            with_hints: vec![],
            version: None,
            with_ordinality: false,
            partitions: vec![],
            json_path: None,
            sample: None,
            index_hints: vec![],
        };
        if let Ok(plan) = ctx.plan(table) {
            return Ok(plan);
        }
        // DataFusion looks up the tables of a statement before it plans the statement, and
        // the input of a table function is not one of them, so the planner looks the table
        // up in the session's catalogs. The lookup is asynchronous and planning is not, so
        // the planner takes the table only if the catalog has it at hand.
        let reference = ctx.object_name_to_table_reference(name)?;
        let options = &ctx.context_provider().options().catalog;
        let full = reference
            .clone()
            .resolve(&options.default_catalog, &options.default_schema);
        let lookup = self
            .catalogs
            .catalog(&full.catalog)
            .and_then(|c| c.schema(&full.schema))
            .map(|s| s.table(&full.table).now_or_never());
        match lookup {
            Some(Some(Ok(Some(table)))) => {
                LogicalPlanBuilder::scan(reference, provider_as_source(table), None)?.build()
            }
            Some(Some(Err(e))) => Err(e),
            Some(None) => Err(err_at(
                span,
                format!(
                    "the catalog of the table {reference} looks it up asynchronously, which the planning of {function} cannot wait for; write (SELECT * FROM {reference})"
                ),
            )),
            None | Some(Some(Ok(None))) => {
                Err(err_at(span, format!("table '{reference}' not found")))
            }
        }
    }

    fn plan_call(
        &self,
        function: &dyn TableFunction,
        ctx: &mut dyn RelationPlannerContext,
        args: Vec<FunctionArg>,
        span: SqlSpan,
    ) -> Result<TableFunctionNode> {
        let name = function.name();
        let mut positional: Vec<SqlExpr> = Vec::new();
        let mut named: Vec<(Ident, SqlExpr)> = Vec::new();
        for arg in args {
            match arg {
                FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => {
                    if let Some((n, _)) = named.last() {
                        return Err(err_at(
                            e.span(),
                            format!(
                                "a positional argument of {name} follows the named argument `{}`",
                                n.value
                            ),
                        ));
                    }
                    positional.push(e);
                }
                FunctionArg::Named {
                    name: arg_name,
                    arg: FunctionArgExpr::Expr(e),
                    ..
                } => {
                    let names: Vec<&str> = function
                        .named()
                        .iter()
                        .copied()
                        .chain(["partition_by"])
                        .collect();
                    if !names.iter().any(|n| arg_name.value.eq_ignore_ascii_case(n)) {
                        return Err(err_at(
                            arg_name.span,
                            format!(
                                "unknown argument `{}` of {name}: the named arguments are {}",
                                arg_name.value,
                                names.join(", ")
                            ),
                        ));
                    }
                    if named
                        .iter()
                        .any(|(n, _)| n.value.eq_ignore_ascii_case(&arg_name.value))
                    {
                        return Err(err_at(
                            arg_name.span,
                            format!("the argument `{}` of {name} is given twice", arg_name.value),
                        ));
                    }
                    named.push((arg_name, e));
                }
                other => {
                    return Err(err_at(
                        other.span(),
                        format!("{name} takes expressions as arguments"),
                    ));
                }
            }
        }
        let wanted: Vec<&str> = std::iter::once("the input")
            .chain(function.positional().iter().copied())
            .collect();
        if positional.len() != wanted.len() {
            let at = positional.get(wanted.len()).map_or(span, |e| e.span());
            let (last, rest) = wanted.split_last().expect("the input");
            let listed = if rest.is_empty() {
                last.to_string()
            } else {
                format!("{}, and {last}", rest.join(", "))
            };
            return Err(err_at(
                at,
                format!(
                    "{name} takes {} positional arguments, {listed}; found {}",
                    wanted.len(),
                    positional.len()
                ),
            ));
        }
        let mut positional = positional.into_iter();
        let input = self.input(ctx, name, positional.next().expect("the input"))?;

        let mut partition_by: Vec<(Column, SqlSpan)> = Vec::new();
        let mut rest = Vec::new();
        for (arg_name, e) in named {
            let arg_name = arg_name.value.to_ascii_lowercase();
            if arg_name != "partition_by" {
                rest.push((arg_name, e));
                continue;
            }
            let exprs = match e {
                SqlExpr::Array(a) => a.elem,
                e => vec![e],
            };
            for e in exprs {
                let at = e.span();
                let c = column(ctx, name, "partition column", e, input.schema())?;
                if partition_by.iter().any(|(p, _)| *p == c) {
                    return Err(err_at(
                        at,
                        format!("the partition column {c} of {name} is given twice"),
                    ));
                }
                partition_by.push((c, at));
            }
        }
        let columns: Vec<Column> = partition_by.iter().map(|(c, _)| c.clone()).collect();
        let transform = function.plan(Call {
            ctx,
            name,
            span,
            input: input.clone(),
            positional: positional.collect(),
            named: rest,
            partition_by: columns.clone(),
        })?;
        let read = transform.columns();
        if let Some((c, at)) = partition_by.iter().find(|(c, _)| read.contains(c)) {
            return Err(err_at(
                *at,
                format!("the partition column {c} of {name} is also a column of another argument"),
            ));
        }
        TableFunctionNode::try_new(input, columns, transform)
    }
}

impl RelationPlanner for TableFunctionPlanner {
    fn plan_relation(
        &self,
        relation: TableFactor,
        ctx: &mut dyn RelationPlannerContext,
    ) -> Result<RelationPlanning> {
        let TableFactor::Table {
            name,
            args: Some(_),
            ..
        } = &relation
        else {
            return Ok(RelationPlanning::Original(Box::new(relation)));
        };
        let called = name.to_string();
        let Some(function) = self
            .functions
            .iter()
            .find(|f| called.eq_ignore_ascii_case(f.name()))
            .cloned()
        else {
            return Ok(RelationPlanning::Original(Box::new(relation)));
        };
        let TableFactor::Table {
            name,
            args: Some(args),
            alias,
            ..
        } = relation
        else {
            unreachable!()
        };
        let node = self.plan_call(function.as_ref(), ctx, args.args, name.span())?;
        let plan = LogicalPlan::Extension(Extension {
            node: Arc::new(node),
        });
        Ok(RelationPlanning::Planned(Box::new(PlannedRelation::new(
            plan, alias,
        ))))
    }
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
