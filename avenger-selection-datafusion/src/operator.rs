//! Selections' operators: the `selected` calls of a plan become operators, which are bound to the
//! selections' states after the plan is built.
//!
//! [`with_selection_nodes`] replaces each `selected` call with a boolean column that a
//! [`SelectionNode`] appends to the call's input. The optimizer treats that column like any other
//! column, so it never moves a selection's test into a scan. The planner plans each node as a
//! [`SelectionExec`], which tests each row against the state of its selection. The operator gets
//! that state in one of two ways: [`SelectionExtensionPlanner`] binds it during planning when the
//! session holds [`SelectionStates`], and [`bind_selections`] binds it in a finished physical plan.

use std::{
    any::Any,
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use async_trait::async_trait;
use avenger_selection::{PreparedPredicate, SelectedOptions, SelectionState};
use datafusion::{
    arrow::{
        array::RecordBatch,
        datatypes::{DataType, Field, Schema, SchemaRef},
    },
    catalog::Session,
    common::{
        exec_err, internal_err, plan_err,
        tree_node::{Transformed, TransformedResult, TreeNode, TreeNodeRecursion},
        Column, ColumnStatistics, DFSchema, DFSchemaRef, Result, Statistics,
    },
    execution::{SendableRecordBatchStream, SessionState, TaskContext},
    logical_expr::{
        expr::{Placeholder, ScalarFunction},
        expr_rewriter::NamePreserver,
        physical_planning_context::PhysicalPlanningContext,
        utils::merge_schema,
        Expr, Extension, LogicalPlan, LogicalPlanBuilder, UserDefinedLogicalNode,
        UserDefinedLogicalNodeCore,
    },
    physical_expr::{EquivalenceProperties, PhysicalExpr},
    physical_plan::{
        statistics::{ChildStats, StatisticsArgs},
        stream::RecordBatchStreamAdapter,
        DisplayAs, DisplayFormatType, ExecutionPlan, ExecutionPlanProperties, PlanProperties,
    },
    physical_planner::{DefaultPhysicalPlanner, ExtensionPlanner, PhysicalPlanner},
};
use futures::StreamExt;

use crate::{
    call::{column, fold_options, is_selected, Options},
    functions::{external, mask},
};

/// The columns that each selection's test can read, keyed by the selection's name without `$`:
/// the columns that the tools that write the selection test. `None` means that the columns are
/// unknown, so the selection's nodes read every column of their inputs.
pub type SelectionFields = BTreeMap<String, Option<Vec<String>>>;

/// The states that a session binds its selections' operators to during planning, keyed by the
/// selections' names without `$`. A session's configuration holds the map as an extension.
#[derive(Debug, Default)]
pub struct SelectionStates(pub BTreeMap<String, SelectionState>);

/// A copy of `state` whose configuration holds `states`, so the session binds each
/// [`SelectionExec`] that it plans to the state of its selection.
pub fn with_selection_states(
    state: &SessionState,
    states: BTreeMap<String, SelectionState>,
) -> SessionState {
    let mut state = state.clone();
    (state.config_mut()).set_extension(Arc::new(SelectionStates(states)));
    state
}

/// Replaces each `selected` call on a placeholder in `plan` with the column of a
/// [`SelectionNode`], which becomes the input of the operator that contains the call. The node's
/// test reads the columns that `fields` names for the selection, or every column of the node's
/// input when `fields` does not name the selection. The result has the schema of `plan`.
///
/// `plan` must be unoptimized, such as from `DataFrame::into_unoptimized_plan`. A `selected` call
/// names no columns, so the optimizer can drop the columns that its test reads. A node names
/// those columns, so an optimization after this function keeps them.
///
/// A call in a join's condition must read the columns of one side of the join. The function
/// fails when `fields` does not name the selection of such a call, or when the selection's
/// columns come from both sides.
pub fn with_selection_nodes(plan: LogicalPlan, fields: &SelectionFields) -> Result<LogicalPlan> {
    plan.transform_up_with_subqueries(|plan| add_nodes(plan, fields))
        .data()
}

/// The placeholder and the arguments of `expr` when `expr` is a `selected` call on a placeholder,
/// such as `$brush`.
fn selected_call(expr: &Expr) -> Option<(&str, &[Expr])> {
    let Expr::ScalarFunction(ScalarFunction { func, args }) = expr else {
        return None;
    };
    match args.first() {
        Some(Expr::Placeholder(Placeholder { id, .. })) if is_selected(func) => Some((id, args)),
        _ => None,
    }
}

/// Replaces the `selected` calls in the expressions of the operator `plan`. An operator that passes
/// its input's columns through, such as a filter, would also output the columns that the nodes
/// add, so a projection above such an operator restores its original columns.
fn add_nodes(plan: LogicalPlan, fields: &SelectionFields) -> Result<Transformed<LogicalPlan>> {
    let mut has_call = false;
    for expr in plan.expressions() {
        expr.apply(|expr| {
            has_call |= selected_call(expr).is_some();
            Ok(match has_call {
                true => TreeNodeRecursion::Stop,
                false => TreeNodeRecursion::Continue,
            })
        })?;
    }
    if !has_call {
        return Ok(Transformed::no(plan));
    }
    let original = plan.schema().clone();
    let schema = merge_schema(&plan.inputs());
    let originals: Vec<LogicalPlan> = plan.inputs().into_iter().cloned().collect();
    let mut inputs = originals.clone();
    let names = NamePreserver::new(&plan);
    let exprs = (plan.expressions().into_iter())
        .map(|expr| {
            let name = names.save(&expr);
            let replaced = expr.transform_up(|expr| {
                replace_call(expr, &schema, fields, &originals, &mut inputs)
            })?;
            Ok(name.restore(replaced.data))
        })
        .collect::<Result<Vec<_>>>()?;
    let rebuilt = plan.with_new_exprs(exprs, inputs)?;
    if rebuilt.schema().fields().len() == original.fields().len() {
        return Ok(Transformed::yes(rebuilt));
    }
    let columns = original.columns().into_iter().map(Expr::Column);
    let projected = LogicalPlanBuilder::from(rebuilt)
        .project(columns)?
        .build()?;
    Ok(Transformed::yes(projected))
}

/// Replaces `expr` with the column of a [`SelectionNode`] when `expr` is a `selected` call on a
/// placeholder.
///
/// `originals` are the inputs of the operator that contains the call, and `schema` is their
/// merged schema, which the call's options are folded against. `inputs` starts as a copy of
/// `originals`, and each call wraps one of them in a node: the input whose columns the call's
/// test reads. An equal call on the same input reuses the node that the earlier call added.
fn replace_call(
    expr: Expr,
    schema: &DFSchema,
    fields: &SelectionFields,
    originals: &[LogicalPlan],
    inputs: &mut [LogicalPlan],
) -> Result<Transformed<Expr>> {
    let Some((id, args)) = selected_call(&expr) else {
        return Ok(Transformed::no(expr));
    };
    let options = Options::parse(&fold_options(args, schema)?)?;
    let partition = (options.partition).filter(|_| options.selected.faceted);
    let wanted = fields.get(id.trim_start_matches('$')).cloned().flatten();
    let position = match originals.len() {
        0 => return plan_err!("`selected` on {id} needs an input to test"),
        1 => 0,
        _ => {
            let Some(wanted) = &wanted else {
                return plan_err!(
                    "`selected` on {id} in a join's condition needs the columns of its selection"
                );
            };
            let mut needed: Vec<String> = wanted.clone();
            if let Some(partition) = &partition {
                needed.extend(partition.column_refs().into_iter().map(|c| c.name.clone()));
            }
            let has = |input: &LogicalPlan| {
                (needed.iter()).all(|name| input.schema().has_column_with_unqualified_name(name))
            };
            match originals.iter().position(has) {
                Some(position) => position,
                None => {
                    return plan_err!(
                        "`selected` on {id} in a join's condition reads the columns of both sides"
                    )
                }
            }
        }
    };
    let input_schema = originals[position].schema().clone();
    let (columns, mut exprs): (Vec<String>, Vec<Expr>) = match wanted {
        // The selection's tools can test a column that this input lacks, such as a column of
        // another plot's table. The node omits that column. A state that does not test the
        // column, such as an empty state, still binds, and a state that tests it fails to bind.
        Some(wanted) => (wanted.into_iter())
            .filter(|name| input_schema.has_column_with_unqualified_name(name))
            .map(|name| Ok((name.clone(), column(&name, &input_schema)?)))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .unzip(),
        None => {
            let mut seen = HashSet::new();
            (input_schema.iter())
                .filter(|(_, field)| seen.insert(field.name().clone()))
                .map(|(qualifier, field)| {
                    let column = Column::new(qualifier.cloned(), field.name());
                    (field.name().clone(), Expr::Column(column))
                })
                .unzip()
        }
    };
    exprs.extend(partition.clone());
    // The column's name is a hash of the call and of the original input. Copies of one subplan,
    // such as the references to a shared input, therefore get the same name and stay equal plans,
    // and an equal call on the same input finds the node that the earlier call added.
    let name = {
        let mut hasher = DefaultHasher::new();
        (id, &options.selected, &columns, &exprs, position).hash(&mut hasher);
        input_schema.hash(&mut hasher);
        format!("__selected_{:016x}", hasher.finish())
    };
    if inputs[position]
        .schema()
        .has_column_with_unqualified_name(&name)
    {
        return Ok(Transformed::yes(Expr::Column(Column::new_unqualified(
            name,
        ))));
    }
    let input = inputs[position].clone();
    let node = SelectionNode::try_new(
        input,
        id.to_string(),
        options.selected,
        columns,
        exprs,
        partition.is_some(),
        name.clone(),
    )?;
    inputs[position] = LogicalPlan::Extension(Extension {
        node: Arc::new(node),
    });
    Ok(Transformed::yes(Expr::Column(Column::new_unqualified(
        name,
    ))))
}

/// A selection's node in a logical plan. The node passes its input's rows through and appends the
/// nullable boolean column `name`, which tells whether the selection whose placeholder is
/// `parameter` selects each row, with the options `options`.
///
/// The node's expressions, `exprs`, are the columns that its test can read, which `columns` names
/// in the same order, then the struct of the row's partition columns when the test is `faceted`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectionNode {
    input: LogicalPlan,
    parameter: String,
    options: SelectedOptions,
    columns: Vec<String>,
    exprs: Vec<Expr>,
    faceted: bool,
    name: String,
    schema: DFSchemaRef,
}

impl SelectionNode {
    fn try_new(
        input: LogicalPlan,
        parameter: String,
        options: SelectedOptions,
        columns: Vec<String>,
        exprs: Vec<Expr>,
        faceted: bool,
        name: String,
    ) -> Result<Self> {
        let schema = schema_with(input.schema(), &name)?;
        Ok(Self {
            input,
            parameter,
            options,
            columns,
            exprs,
            faceted,
            name,
            schema,
        })
    }
}

/// `input` with the nullable boolean column `name` after its columns.
fn schema_with(input: &DFSchemaRef, name: &str) -> Result<DFSchemaRef> {
    let field = Field::new(name, DataType::Boolean, true);
    let added = DFSchema::from_unqualified_fields(vec![field].into(), HashMap::new())?;
    Ok(Arc::new(input.join(&added)?))
}

impl PartialOrd for SelectionNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (self == other).then_some(Ordering::Equal)
    }
}

impl UserDefinedLogicalNodeCore for SelectionNode {
    fn name(&self) -> &str {
        "Selection"
    }

    fn inputs(&self) -> Vec<&LogicalPlan> {
        vec![&self.input]
    }

    fn schema(&self) -> &DFSchemaRef {
        &self.schema
    }

    fn expressions(&self) -> Vec<Expr> {
        self.exprs.clone()
    }

    fn prevent_predicate_push_down_columns(&self) -> HashSet<String> {
        HashSet::from([self.name.clone()])
    }

    fn fmt_for_explain(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Selection: {} AS {} over [{}]",
            self.parameter,
            self.name,
            self.columns.join(", ")
        )
    }

    fn with_exprs_and_inputs(&self, exprs: Vec<Expr>, inputs: Vec<LogicalPlan>) -> Result<Self> {
        let Ok([input]) = <[LogicalPlan; 1]>::try_from(inputs) else {
            return internal_err!("a selection's node has one input");
        };
        Self::try_new(
            input,
            self.parameter.clone(),
            self.options.clone(),
            self.columns.clone(),
            exprs,
            self.faceted,
            self.name.clone(),
        )
    }

    fn necessary_children_exprs(&self, output_columns: &[usize]) -> Option<Vec<Vec<usize>>> {
        let schema = self.input.schema();
        let width = schema.fields().len();
        let mut needed: Vec<usize> = (output_columns.iter().copied())
            .filter(|index| *index < width)
            .collect();
        for expr in &self.exprs {
            for column in expr.column_refs() {
                needed.push(schema.index_of_column(column).ok()?);
            }
        }
        needed.sort_unstable();
        needed.dedup();
        Some(vec![needed])
    }

    fn supports_limit_pushdown(&self) -> bool {
        true
    }
}

/// A selection's operator in a physical plan. The operator passes its input's batches through and
/// appends a boolean column, which tells whether the state of its selection selects each row.
///
/// The operator runs only after it is bound to a state, by [`bind_selections`] or by a planning
/// session that holds [`SelectionStates`]. An unbound operator fails when it runs.
pub struct SelectionExec {
    input: Arc<dyn ExecutionPlan>,
    parameter: String,
    options: SelectedOptions,
    columns: Vec<String>,
    exprs: Vec<Arc<dyn PhysicalExpr>>,
    faceted: bool,
    name: String,
    test: Option<Arc<PreparedPredicate>>,
    schema: SchemaRef,
    properties: Arc<PlanProperties>,
}

impl SelectionExec {
    /// An unbound operator over `input` for the selection whose placeholder is `parameter`, which
    /// appends the column `name`. `options` are the options of its test. `columns` names the
    /// columns that the test can read, and `exprs` holds their expressions in the same order, then
    /// the struct of the row's partition columns when the test is `faceted`.
    pub fn try_new(
        input: Arc<dyn ExecutionPlan>,
        parameter: String,
        options: SelectedOptions,
        columns: Vec<String>,
        exprs: Vec<Arc<dyn PhysicalExpr>>,
        faceted: bool,
        name: String,
    ) -> Result<Self> {
        let input_schema = input.schema();
        let mut fields: Vec<_> = input_schema.fields().iter().cloned().collect();
        fields.push(Arc::new(Field::new(&name, DataType::Boolean, true)));
        let schema = Arc::new(Schema::new_with_metadata(
            fields,
            input_schema.metadata().clone(),
        ));
        let properties = properties(&input, &schema);
        Ok(Self {
            input,
            parameter,
            options,
            columns,
            exprs,
            faceted,
            name,
            test: None,
            schema,
            properties,
        })
    }

    /// The placeholder of the operator's selection, such as `$brush`.
    pub fn parameter(&self) -> &str {
        &self.parameter
    }

    /// The options of the operator's test.
    pub fn options(&self) -> &SelectedOptions {
        &self.options
    }

    /// The names of the columns that the operator's test can read.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// The expressions of the test's columns, then of the struct of the row's partition columns
    /// when the test is faceted.
    pub fn exprs(&self) -> &[Arc<dyn PhysicalExpr>] {
        &self.exprs
    }

    /// Whether the test reads the struct of the row's partition columns.
    pub fn faceted(&self) -> bool {
        self.faceted
    }

    /// The name of the operator's column.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// A copy of the operator that is bound to `state`, with its test prepared for the types of
    /// its columns. Fails when `state` tests a column that the operator does not read.
    pub fn bound(&self, state: &SelectionState) -> Result<Self> {
        let input_schema = self.input.schema();
        let mut types = BTreeMap::new();
        for (name, expr) in self.columns.iter().zip(&self.exprs) {
            types.insert(name.as_str(), expr.data_type(&input_schema)?);
        }
        let prepared = state
            .prepare_selected(&self.options, |name| types.get(name).cloned())
            .map_err(external)?;
        if let Some(name) = (prepared.columns().into_iter())
            .find(|name| !self.columns.iter().any(|column| column == name))
        {
            return plan_err!(
                "the selection {} tests the column `{name}`, which its operator does not read",
                self.parameter
            );
        }
        Ok(Self {
            test: Some(Arc::new(prepared)),
            ..self.with_input(self.input.clone())
        })
    }

    /// A copy of the operator over `input`, with the same test.
    fn with_input(&self, input: Arc<dyn ExecutionPlan>) -> Self {
        let properties = properties(&input, &self.schema);
        Self {
            input,
            parameter: self.parameter.clone(),
            options: self.options.clone(),
            columns: self.columns.clone(),
            exprs: self.exprs.clone(),
            faceted: self.faceted,
            name: self.name.clone(),
            test: self.test.clone(),
            schema: self.schema.clone(),
            properties,
        }
    }
}

/// The plan properties of a selection's operator with `schema` over `input`. The operator keeps
/// every row in its place, so it has the input's partitioning, orderings, emission, and
/// boundedness.
fn properties(input: &Arc<dyn ExecutionPlan>, schema: &SchemaRef) -> Arc<PlanProperties> {
    let orderings = input.equivalence_properties().oeq_class().iter().cloned();
    let input = input.properties();
    Arc::new(PlanProperties::new(
        EquivalenceProperties::new_with_orderings(schema.clone(), orderings),
        input.output_partitioning().clone(),
        input.emission_type,
        input.boundedness,
    ))
}

impl fmt::Debug for SelectionExec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (f.debug_struct("SelectionExec"))
            .field("parameter", &self.parameter)
            .field("name", &self.name)
            .field("bound", &self.test.is_some())
            .finish()
    }
}

impl DisplayAs for SelectionExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "SelectionExec: {} AS {} over [{}]",
            self.parameter,
            self.name,
            self.columns.join(", ")
        )?;
        if self.test.is_none() {
            write!(f, ", unbound")?;
        }
        Ok(())
    }
}

impl ExecutionPlan for SelectionExec {
    fn name(&self) -> &str {
        "SelectionExec"
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }

    fn maintains_input_order(&self) -> Vec<bool> {
        vec![true]
    }

    fn child_stats_requests(&self, partition: Option<usize>) -> Vec<ChildStats> {
        vec![ChildStats::At(partition)]
    }

    fn statistics_from_inputs(
        &self,
        input_stats: &[Arc<Statistics>],
        _args: &StatisticsArgs,
    ) -> Result<Arc<Statistics>> {
        let mut statistics = input_stats[0].as_ref().clone();
        (statistics.column_statistics).push(ColumnStatistics::new_unknown());
        Ok(Arc::new(statistics))
    }

    fn supports_limit_pushdown(&self) -> bool {
        true
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![&self.input]
    }

    fn apply_expressions(
        &self,
        f: &mut dyn FnMut(&Arc<dyn PhysicalExpr>) -> Result<TreeNodeRecursion>,
    ) -> Result<TreeNodeRecursion> {
        for expr in &self.exprs {
            if f(expr)? == TreeNodeRecursion::Stop {
                return Ok(TreeNodeRecursion::Stop);
            }
        }
        Ok(TreeNodeRecursion::Continue)
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        match <[_; 1]>::try_from(children) {
            Ok([input]) => Ok(Arc::new(self.with_input(input))),
            Err(_) => internal_err!("a selection's operator has one input"),
        }
    }

    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        let Some(test) = self.test.clone() else {
            return exec_err!(
                "the selection {} is not bound to a state: bind the plan before it runs",
                self.parameter
            );
        };
        let input = self.input.execute(partition, context)?;
        let schema = self.schema.clone();
        let exprs = self.exprs.clone();
        let columns = self.columns.clone();
        let faceted = self.faceted;
        let batches = input.map(move |batch| {
            let batch = batch?;
            let rows = batch.num_rows();
            let arrays = (exprs.iter())
                .map(|expr| expr.evaluate(&batch)?.into_array(rows))
                .collect::<Result<Vec<_>>>()?;
            let selected = mask(&test, &columns, &arrays, faceted, rows)?;
            let mut columns = batch.columns().to_vec();
            columns.push(Arc::new(selected));
            Ok(RecordBatch::try_new(schema.clone(), columns)?)
        });
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            self.schema.clone(),
            batches,
        )))
    }
}

/// Binds each [`SelectionExec`] in `plan` to the state of its selection in `states`, which is
/// keyed by the selections' names without `$`. Fails when `states` lacks a selection of the plan.
///
/// A host that plans a query once binds the plan for each run. The host then passes the bound
/// plan to `reset_plan_states`, so the run's operators share no execution state with an earlier
/// run.
pub fn bind_selections(
    plan: Arc<dyn ExecutionPlan>,
    states: &BTreeMap<String, SelectionState>,
) -> Result<Arc<dyn ExecutionPlan>> {
    plan.transform_up(|node| {
        let Some(selection) = node.downcast_ref::<SelectionExec>() else {
            return Ok(Transformed::no(node));
        };
        let Some(state) = states.get(selection.parameter.trim_start_matches('$')) else {
            return plan_err!("no state for the selection {}", selection.parameter);
        };
        let bound: Arc<dyn ExecutionPlan> = Arc::new(selection.bound(state)?);
        Ok(Transformed::yes(bound))
    })
    .data()
}

/// Plans each [`SelectionNode`] as a [`SelectionExec`]. The operator is bound when the session's
/// configuration holds [`SelectionStates`] with a state for its selection, and unbound otherwise.
#[derive(Debug, Default)]
pub struct SelectionExtensionPlanner;

#[async_trait]
impl ExtensionPlanner for SelectionExtensionPlanner {
    async fn plan_extension(
        &self,
        planner: &dyn PhysicalPlanner,
        node: &dyn UserDefinedLogicalNode,
        logical_inputs: &[&LogicalPlan],
        physical_inputs: &[Arc<dyn ExecutionPlan>],
        session: &dyn Session,
        ctx: &PhysicalPlanningContext,
    ) -> Result<Option<Arc<dyn ExecutionPlan>>> {
        let Some(node) = (node.as_any() as &dyn Any).downcast_ref::<SelectionNode>() else {
            return Ok(None);
        };
        let schema = logical_inputs[0].schema();
        let exprs = (node.exprs.iter())
            .map(|expr| planner.create_physical_expr(expr, schema, session, ctx))
            .collect::<Result<Vec<_>>>()?;
        let operator = SelectionExec::try_new(
            physical_inputs[0].clone(),
            node.parameter.clone(),
            node.options.clone(),
            node.columns.clone(),
            exprs,
            node.faceted,
            node.name.clone(),
        )?;
        let states = session.config().get_extension::<SelectionStates>();
        let state = (states.as_ref()).and_then(|states| {
            let name = node.parameter.trim_start_matches('$');
            states.0.get(name).cloned()
        });
        Ok(Some(match state {
            Some(state) => Arc::new(operator.bound(&state)?),
            None => Arc::new(operator),
        }))
    }
}

/// DataFusion's default query planner with [`SelectionExtensionPlanner`], for a host that has no
/// query planner of its own.
#[derive(Debug)]
pub struct QueryPlanner;

#[async_trait]
impl datafusion::execution::context::QueryPlanner for QueryPlanner {
    async fn create_physical_plan(
        &self,
        plan: &LogicalPlan,
        session: &dyn Session,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        DefaultPhysicalPlanner::with_extension_planners(vec![Arc::new(SelectionExtensionPlanner)])
            .create_physical_plan(plan, session)
            .await
    }
}
