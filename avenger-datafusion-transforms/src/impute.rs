//! `impute`: a table function that adds the rows that each group lacks.
//!
//! ```sql
//! SELECT * FROM impute((SELECT month, region, sum(units) AS units FROM sales GROUP BY 1, 2),
//!                      month, [region], units, value => 0, flag => imputed)
//! ```
//!
//! The first argument is the input, a subquery. A table name works only for a table that the
//! statement reads elsewhere, because DataFusion resolves the tables of a statement before it
//! plans the statement. Then come the key column, the group columns (a list, `[]`, or one
//! column without brackets), and the value column. The named arguments are `value`, `method` (`'mean'`, `'median'`, `'min'`, `'max'`, or
//! `'gap'`), `frame => [lo, hi]`, `keys => [...]`, `extra_keys => [...]`, and `flag => name`.
//!
//! For each group (the distinct combinations of the group columns) and each key value, the
//! function adds a row when the group has no row with that key. The key values are `keys`,
//! or the distinct non-null keys of the input, plus `extra_keys`. Added rows have the key,
//! the group columns, and the imputed value; their other columns are NULL. Existing rows
//! stay as they are. `flag` adds a boolean column that is true in added rows.
//!
//! The key values come from the input and the arguments only: the bins of a bin key that no
//! row has, and the units of a time unit key that no row has, are not key values. Added rows
//! follow the input rows, by group in the order in which the groups first occur, then in key
//! order.
//!
//! Three parts:
//!
//! - [`ImputePlanner`], a relation planner: it plans the input, binds the arguments to the
//!   input's columns, checks them, and reports errors at the arguments' places.
//! - [`ImputeNode`], a logical extension node.
//! - [`ImputeExec`], which reads its input once, into memory, then emits the input rows and
//!   the added rows. The same result from SQL joins would read the input once for the keys,
//!   once for the groups, and once for the rows: DataFusion does not share a subplan between
//!   readers, and the input of `impute` is usually an aggregate.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, BooleanArray, Float64Array, RecordBatch, RecordBatchOptions,
    UInt32Array, new_null_array,
};
use datafusion::arrow::compute::{cast, concat, concat_batches, take};
use datafusion::arrow::datatypes::{DataType, Field, Float64Type, Schema, SchemaRef};
use datafusion::arrow::row::{RowConverter, SortField};
use datafusion::catalog::Session;
use datafusion::common::tree_node::TreeNodeRecursion;
use datafusion::common::{
    Column, DFSchema, DFSchemaRef, DataFusionError, Diagnostic, ExprSchema, Result, ScalarValue,
    Span,
};
use datafusion::execution::context::QueryPlanner;
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
use datafusion::physical_expr::{EquivalenceProperties, PhysicalExpr, create_physical_expr};
use datafusion::physical_plan::execution_plan::EmissionType;
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::{
    ChildrenPropertiesMode, DisplayAs, DisplayFormatType, Distribution, ExecutionPlan,
    InputDistributionRequirements, Partitioning, PlanProperties, ReplaceChildrenOptions,
};
use datafusion::physical_planner::{DefaultPhysicalPlanner, ExtensionPlanner, PhysicalPlanner};
use datafusion::sql::sqlparser::ast::{
    Expr as SqlExpr, FunctionArg, FunctionArgExpr, Ident, ObjectName, Spanned, TableFactor, Value,
};
use datafusion::sql::sqlparser::tokenizer::Span as SqlSpan;
use futures::{StreamExt, TryStreamExt};

// ---------------------------------------------------------------------------------------
// The fill
// ---------------------------------------------------------------------------------------

/// A statistic of the existing values of a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Hash)]
pub enum Stat {
    Mean,
    Median,
    Min,
    Max,
}

impl Stat {
    fn of(self, values: &mut [f64]) -> Option<f64> {
        if values.is_empty() {
            return None;
        }
        Some(match self {
            Stat::Mean => values.iter().sum::<f64>() / values.len() as f64,
            Stat::Median => {
                values.sort_by(f64::total_cmp);
                let m = values.len() / 2;
                if values.len() % 2 == 1 {
                    values[m]
                } else {
                    (values[m - 1] + values[m]) / 2.0
                }
            }
            // In the total order of DataFusion's `min` and `max`, NaN is the greatest.
            Stat::Min => values.iter().copied().min_by(f64::total_cmp)?,
            Stat::Max => values.iter().copied().max_by(f64::total_cmp)?,
        })
    }
}

/// The value of added rows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Hash)]
pub enum Fill {
    /// `value => v`, already of the type of the value column.
    Value(ScalarValue),
    /// `method => 'gap'`: NULL.
    Gap,
    /// `method => 'mean'` and the others, over the whole group or over `frame => [lo, hi]`
    /// (None is an open end).
    Stat {
        stat: Stat,
        frame: Option<(Option<i64>, Option<i64>)>,
    },
}

// ---------------------------------------------------------------------------------------
// The logical node
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImputeNode {
    pub input: LogicalPlan,
    pub key: Column,
    pub groupby: Vec<Column>,
    pub val: Column,
    pub fill: Fill,
    /// `keys => [...]`, of the type of the key.
    pub keys: Option<Vec<ScalarValue>>,
    /// `extra_keys => [...]`, of the type of the key.
    pub extra_keys: Vec<ScalarValue>,
    pub flag: Option<String>,
    pub schema: DFSchemaRef,
}

impl PartialOrd for ImputeNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (self == other).then_some(Ordering::Equal)
    }
}

impl ImputeNode {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        input: LogicalPlan,
        key: Column,
        groupby: Vec<Column>,
        val: Column,
        fill: Fill,
        keys: Option<Vec<ScalarValue>>,
        extra_keys: Vec<ScalarValue>,
        flag: Option<String>,
    ) -> Result<Self> {
        let schema = output_schema(input.schema(), &key, &groupby, flag.as_deref())?;
        Ok(Self {
            input,
            key,
            groupby,
            val,
            fill,
            keys,
            extra_keys,
            flag,
            schema,
        })
    }
}

/// The input columns, then the flag. Added rows are NULL in the columns other than the
/// key and the groups, so those columns become nullable.
fn output_schema(
    input: &DFSchema,
    key: &Column,
    groupby: &[Column],
    flag: Option<&str>,
) -> Result<DFSchemaRef> {
    let key_i = input.index_of_column(key)?;
    let group_i = groupby
        .iter()
        .map(|c| input.index_of_column(c))
        .collect::<Result<HashSet<_>>>()?;
    let mut fields: Vec<_> = input
        .iter()
        .enumerate()
        .map(|(i, (q, f))| {
            let keep = i == key_i || group_i.contains(&i);
            let f = if keep {
                f.clone()
            } else {
                Arc::new(f.as_ref().clone().with_nullable(true))
            };
            (q.cloned(), f)
        })
        .collect();
    if let Some(flag) = flag {
        fields.push((None, Arc::new(Field::new(flag, DataType::Boolean, false))));
    }
    Ok(Arc::new(DFSchema::new_with_metadata(
        fields,
        input.metadata().clone(),
    )?))
}

impl UserDefinedLogicalNodeCore for ImputeNode {
    fn name(&self) -> &str {
        "Impute"
    }
    fn inputs(&self) -> Vec<&LogicalPlan> {
        vec![&self.input]
    }
    fn schema(&self) -> &DFSchemaRef {
        &self.schema
    }
    fn expressions(&self) -> Vec<Expr> {
        let mut exprs = vec![Expr::Column(self.key.clone())];
        exprs.extend(self.groupby.iter().cloned().map(Expr::Column));
        exprs.push(Expr::Column(self.val.clone()));
        exprs
    }
    fn fmt_for_explain(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let groups: Vec<_> = self.groupby.iter().map(|c| c.to_string()).collect();
        write!(
            f,
            "Impute: key={}, groupby=[{}], val={}, fill={:?}",
            self.key,
            groups.join(", "),
            self.val,
            self.fill
        )?;
        if let Some(keys) = &self.keys {
            write!(f, ", keys={}", keys.len())?;
        }
        if !self.extra_keys.is_empty() {
            write!(f, ", extra_keys={}", self.extra_keys.len())?;
        }
        if let Some(flag) = &self.flag {
            write!(f, ", flag={flag}")?;
        }
        Ok(())
    }
    fn with_exprs_and_inputs(&self, exprs: Vec<Expr>, inputs: Vec<LogicalPlan>) -> Result<Self> {
        let columns = exprs
            .into_iter()
            .map(|e| match e {
                Expr::Column(c) => Ok(c),
                e => datafusion::common::internal_err!("Impute expects columns, not {e}"),
            })
            .collect::<Result<Vec<_>>>()?;
        let n = columns.len();
        let input = inputs.into_iter().next().expect("one input");
        // A rebuild over an input with more columns, such as DataFusion's pushdown of a struct
        // field access (`key['month']`) into the input: the added rows would be NULL in those
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
        Self::try_new(
            input,
            columns[0].clone(),
            columns[1..n - 1].to_vec(),
            columns[n - 1].clone(),
            self.fill.clone(),
            self.keys.clone(),
            self.extra_keys.clone(),
            self.flag.clone(),
        )
    }
}

// ---------------------------------------------------------------------------------------
// The relation planner
// ---------------------------------------------------------------------------------------

/// Plans `impute(...)` in `FROM`.
#[derive(Debug)]
pub struct ImputePlanner;

/// An error at the place of a part of the query.
fn err_at(span: SqlSpan, message: impl Into<String>) -> DataFusionError {
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
fn constant(expr: &Expr) -> Result<ScalarValue> {
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
fn cast_exact(v: &ScalarValue, to: &DataType) -> Option<ScalarValue> {
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

/// A list of constants, each cast to `data_type`.
fn constant_list(
    ctx: &mut dyn RelationPlannerContext,
    name: &str,
    e: SqlExpr,
    data_type: &DataType,
) -> Result<Vec<ScalarValue>> {
    let span = e.span();
    let SqlExpr::Array(array) = e else {
        return Err(err_at(
            span,
            format!("`{name}` must be a list of constants, such as [1, 2]"),
        ));
    };
    let mut out: Vec<ScalarValue> = Vec::new();
    for elem in array.elem {
        let span = elem.span();
        let not_constant = |e: DataFusionError| {
            err_at(
                span,
                format!(
                    "a value of `{name}` must be a constant: {}",
                    e.strip_backtrace()
                ),
            )
        };
        let expr = ctx
            .sql_to_expr(elem, &DFSchema::empty())
            .map_err(not_constant)?;
        let v = constant(&expr).map_err(not_constant)?;
        if v.is_null() {
            return Err(err_at(span, format!("a value of `{name}` cannot be NULL")));
        }
        let v = cast_exact(&v, data_type).ok_or_else(|| {
            let shown = if v.try_as_str().is_some() { format!("'{v}'") } else { v.to_string() };
            err_at(span, format!("the value {shown} of `{name}` does not coerce to the type of the key, {data_type}"))
        })?;
        if !out.contains(&v) {
            out.push(v);
        }
    }
    Ok(out)
}

/// A column of the input.
fn column(
    ctx: &mut dyn RelationPlannerContext,
    what: &str,
    e: SqlExpr,
    schema: &DFSchema,
) -> Result<Column> {
    let span = e.span();
    match ctx.sql_to_expr(e, schema) {
        Ok(Expr::Column(c)) => Ok(c),
        Ok(other) => Err(err_at(
            span,
            format!("the {what} of impute must be a column of its input, not {other}"),
        )),
        Err(e) => Err(err_at(
            span,
            format!(
                "the {what} of impute must be a column of its input: {}",
                e.strip_backtrace().lines().next().unwrap_or("")
            ),
        )),
    }
}

fn is_named(name: &Ident, wanted: &str) -> bool {
    name.value.eq_ignore_ascii_case(wanted)
}

impl RelationPlanner for ImputePlanner {
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
        if !name.to_string().eq_ignore_ascii_case("impute") {
            return Ok(RelationPlanning::Original(Box::new(relation)));
        }
        let TableFactor::Table {
            name,
            args: Some(args),
            alias,
            ..
        } = relation
        else {
            unreachable!()
        };
        let call_span = name.span();
        let node = plan_impute(ctx, args.args, call_span)?;
        let plan = LogicalPlan::Extension(Extension {
            node: Arc::new(node),
        });
        Ok(RelationPlanning::Planned(Box::new(PlannedRelation::new(
            plan, alias,
        ))))
    }
}

fn plan_impute(
    ctx: &mut dyn RelationPlannerContext,
    args: Vec<FunctionArg>,
    call_span: SqlSpan,
) -> Result<ImputeNode> {
    // Split the positional and the named arguments.
    let mut positional: Vec<SqlExpr> = Vec::new();
    let mut named: Vec<(Ident, SqlExpr)> = Vec::new();
    for arg in args {
        match arg {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => {
                if let Some((n, _)) = named.last() {
                    return Err(err_at(
                        e.span(),
                        format!(
                            "a positional argument of impute follows the named argument `{}`",
                            n.value
                        ),
                    ));
                }
                positional.push(e);
            }
            FunctionArg::Named {
                name,
                arg: FunctionArgExpr::Expr(e),
                ..
            } => {
                const NAMES: [&str; 6] = ["value", "method", "frame", "keys", "extra_keys", "flag"];
                if !NAMES.iter().any(|n| is_named(&name, n)) {
                    return Err(err_at(
                        name.span,
                        format!(
                            "unknown argument `{}` of impute: the named arguments are {}",
                            name.value,
                            NAMES.join(", ")
                        ),
                    ));
                }
                if named
                    .iter()
                    .any(|(n, _)| n.value.eq_ignore_ascii_case(&name.value))
                {
                    return Err(err_at(
                        name.span,
                        format!("the argument `{}` of impute is given twice", name.value),
                    ));
                }
                named.push((name, e));
            }
            other => {
                return Err(err_at(
                    other.span(),
                    "impute takes expressions as arguments",
                ));
            }
        }
    }
    if positional.len() != 4 {
        let span = positional.get(4).map_or(call_span, |e| e.span());
        return Err(err_at(
            span,
            format!(
                "impute takes 4 positional arguments, the input, the key, the groups, and the value column; found {}",
                positional.len()
            ),
        ));
    }
    let mut positional = positional.into_iter();
    let input_expr = positional.next().unwrap();
    let key_expr = positional.next().unwrap();
    let group_expr = positional.next().unwrap();
    let val_expr = positional.next().unwrap();

    // The input: a table name or a subquery.
    let input_span = input_expr.span();
    let input = match input_expr {
        SqlExpr::Subquery(query) => ctx.plan(TableFactor::Derived {
            lateral: false,
            subquery: query,
            alias: None,
            sample: None,
        })?,
        SqlExpr::Identifier(id) => table_scan(ctx, ObjectName::from(vec![id]))?,
        SqlExpr::CompoundIdentifier(ids) => table_scan(ctx, ObjectName::from(ids))?,
        _ => {
            return Err(err_at(
                input_span,
                "the input of impute must be a table name or a subquery",
            ));
        }
    };
    let schema = input.schema().clone();

    let key = column(ctx, "key", key_expr, &schema)?;
    let group_exprs = match group_expr {
        SqlExpr::Array(a) => a.elem,
        e => vec![e],
    };
    let mut groupby = Vec::new();
    for e in group_exprs {
        let span = e.span();
        let c = column(ctx, "group", e, &schema)?;
        if c == key {
            return Err(err_at(
                span,
                format!("the key {c} of impute is also a group column"),
            ));
        }
        if groupby.contains(&c) {
            return Err(err_at(
                span,
                format!("the group column {c} of impute is given twice"),
            ));
        }
        groupby.push(c);
    }
    let val_span = val_expr.span();
    let val = column(ctx, "value column", val_expr, &schema)?;
    if val == key || groupby.contains(&val) {
        return Err(err_at(
            val_span,
            format!("the value column {val} of impute is also the key or a group column"),
        ));
    }
    let key_type = schema.field_from_column(&key)?.data_type().clone();
    let val_type = schema.field_from_column(&val)?.data_type().clone();

    // The named arguments.
    let mut value = None;
    let mut method: Option<(String, SqlSpan)> = None;
    let mut frame = None;
    let mut keys = None;
    let mut extra_keys = Vec::new();
    let mut flag = None;
    for (name, e) in named {
        let span = e.span();
        let name = name.value.to_ascii_lowercase();
        match name.as_str() {
            "value" => {
                let not_constant = |e: DataFusionError| {
                    err_at(
                        span,
                        format!("`value` must be a constant: {}", e.strip_backtrace()),
                    )
                };
                let expr = ctx
                    .sql_to_expr(e, &DFSchema::empty())
                    .map_err(not_constant)?;
                let v = constant(&expr).map_err(not_constant)?;
                let Some(cast) = cast_exact(&v, &val_type) else {
                    let shown = if v.data_type().is_null() || v.try_as_str().is_none() {
                        v.to_string()
                    } else {
                        format!("'{v}'")
                    };
                    return Err(err_at(
                        span,
                        format!("`value` {shown} does not coerce to the type of {val}, {val_type}"),
                    ));
                };
                value = Some((cast, span));
            }
            "method" => {
                let SqlExpr::Value(v) = &e else {
                    return Err(err_at(
                        span,
                        "`method` must be 'mean', 'median', 'min', 'max', or 'gap'",
                    ));
                };
                let Value::SingleQuotedString(s) = &v.value else {
                    return Err(err_at(
                        span,
                        format!(
                            "`method` must be 'mean', 'median', 'min', 'max', or 'gap'; found {e}"
                        ),
                    ));
                };
                method = Some((s.clone(), span));
            }
            "frame" => {
                let SqlExpr::Array(a) = e else {
                    return Err(err_at(
                        span,
                        "`frame` must be a list of two bounds, such as [-2, 2]",
                    ));
                };
                if a.elem.len() != 2 {
                    return Err(err_at(
                        span,
                        "`frame` must be a list of two bounds, such as [-2, 2]",
                    ));
                }
                let mut bounds = [None, None];
                for (i, b) in a.elem.into_iter().enumerate() {
                    let bspan = b.span();
                    let expr = ctx.sql_to_expr(b, &DFSchema::empty()).map_err(|_| {
                        err_at(bspan, "a bound of `frame` must be an integer or NULL")
                    })?;
                    bounds[i] = match constant(&expr) {
                        Ok(v) if v.is_null() => None,
                        Ok(v) if v.data_type().is_integer() => match v.cast_to(&DataType::Int64) {
                            Ok(ScalarValue::Int64(Some(b))) => Some(b),
                            _ => {
                                return Err(err_at(
                                    bspan,
                                    "a bound of `frame` must fit in a BIGINT",
                                ));
                            }
                        },
                        _ => {
                            return Err(err_at(
                                bspan,
                                "a bound of `frame` must be an integer or NULL",
                            ));
                        }
                    };
                }
                if let [Some(lo), Some(hi)] = bounds
                    && lo > hi
                {
                    return Err(err_at(
                        span,
                        format!("the start of `frame`, {lo}, is after its end, {hi}"),
                    ));
                }
                frame = Some(((bounds[0], bounds[1]), span));
            }
            "keys" => keys = Some(constant_list(ctx, "keys", e, &key_type)?),
            "extra_keys" => extra_keys = constant_list(ctx, "extra_keys", e, &key_type)?,
            "flag" => {
                let SqlExpr::Identifier(id) = e else {
                    return Err(err_at(
                        span,
                        "`flag` must be a column name, such as `flag => imputed`",
                    ));
                };
                let name = ctx.normalize_ident(id);
                if schema.fields().iter().any(|f| f.name() == &name) {
                    return Err(err_at(
                        span,
                        format!("the flag column `{name}` is already a column of the input"),
                    ));
                }
                flag = Some(name);
            }
            _ => unreachable!("checked above"),
        }
    }

    let fill = match (value, method) {
        (Some(_), Some((_, span))) => {
            return Err(err_at(
                span,
                "impute needs exactly one of `value` and `method`; found both",
            ));
        }
        (None, None) => {
            return Err(err_at(
                call_span,
                "impute needs exactly one of `value` and `method`; found neither",
            ));
        }
        (Some((v, _)), None) => {
            if let Some((_, span)) = frame {
                return Err(err_at(
                    span,
                    "`frame` needs a method of 'mean', 'median', 'min', or 'max'; found `value`",
                ));
            }
            Fill::Value(v)
        }
        (None, Some((m, span))) => {
            let stat = match m.as_str() {
                "gap" => {
                    if let Some((_, fspan)) = frame {
                        return Err(err_at(
                            fspan,
                            "`frame` needs a method of 'mean', 'median', 'min', or 'max'; found 'gap'",
                        ));
                    }
                    return ImputeNode::try_new(
                        input,
                        key,
                        groupby,
                        val,
                        Fill::Gap,
                        keys,
                        extra_keys,
                        flag,
                    );
                }
                "mean" => Stat::Mean,
                "median" => Stat::Median,
                "min" => Stat::Min,
                "max" => Stat::Max,
                _ => {
                    return Err(err_at(
                        span,
                        format!(
                            "`method` must be 'mean', 'median', 'min', 'max', or 'gap'; found '{m}'"
                        ),
                    ));
                }
            };
            if !val_type.is_numeric() {
                return Err(err_at(
                    span,
                    format!("`method => '{m}'` needs a numeric value column; {val} is {val_type}"),
                ));
            }
            Fill::Stat {
                stat,
                frame: frame.map(|(f, _)| f),
            }
        }
    };
    ImputeNode::try_new(input, key, groupby, val, fill, keys, extra_keys, flag)
}

fn table_scan(ctx: &mut dyn RelationPlannerContext, name: ObjectName) -> Result<LogicalPlan> {
    let span = name.span();
    let reference = ctx.object_name_to_table_reference(name)?;
    let source = ctx
        .context_provider()
        .get_table_source(reference.clone())
        .map_err(|_| {
            err_at(span, format!(
                "impute cannot read the table {reference} by name: DataFusion resolves the tables of a \
                 statement before planning it, and a name in an argument is not one of them; \
                 write (SELECT * FROM {reference})"
            ))
        })?;
    LogicalPlanBuilder::scan(reference, source, None)?.build()
}

// ---------------------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct ImputeExec {
    input: Arc<dyn ExecutionPlan>,
    key: usize,
    groups: Vec<usize>,
    val: usize,
    fill: Fill,
    keys: Option<Vec<ScalarValue>>,
    extra_keys: Vec<ScalarValue>,
    schema: SchemaRef,
    properties: Arc<PlanProperties>,
}

impl ImputeExec {
    #[allow(clippy::too_many_arguments)]
    fn new(
        input: Arc<dyn ExecutionPlan>,
        key: usize,
        groups: Vec<usize>,
        val: usize,
        fill: Fill,
        keys: Option<Vec<ScalarValue>>,
        extra_keys: Vec<ScalarValue>,
        schema: SchemaRef,
    ) -> Self {
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(schema.clone()),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Final,
            input.properties().boundedness,
        ));
        Self {
            input,
            key,
            groups,
            val,
            fill,
            keys,
            extra_keys,
            schema,
            properties,
        }
    }
}

impl DisplayAs for ImputeExec {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "ImputeExec: key={}, groups={:?}, val={}, fill={:?}",
            self.key, self.groups, self.val, self.fill
        )
    }
}

impl ExecutionPlan for ImputeExec {
    fn name(&self) -> &str {
        "ImputeExec"
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }
    fn input_distribution_requirements(&self) -> InputDistributionRequirements {
        InputDistributionRequirements::new(vec![Distribution::SinglePartition])
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
            self.key,
            self.groups.clone(),
            self.val,
            self.fill.clone(),
            self.keys.clone(),
            self.extra_keys.clone(),
            self.schema.clone(),
        )))
    }
    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        let batch_size = context.session_config().batch_size().max(1);
        let input = self.input.execute(partition, context)?;
        let input_schema = self.input.schema();
        let schema = self.schema.clone();
        let (key, groups, val) = (self.key, self.groups.clone(), self.val);
        let (fill, keys, extra_keys) = (
            self.fill.clone(),
            self.keys.clone(),
            self.extra_keys.clone(),
        );
        let out = schema.clone();
        let stream = futures::stream::once(async move {
            let batches: Vec<RecordBatch> = input.try_collect().await?;
            let input = concat_batches(&input_schema, &batches)?;
            impute_batch(
                &input,
                key,
                &groups,
                val,
                &fill,
                keys.as_deref(),
                &extra_keys,
                out,
            )
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

/// Owned row bytes, to hash and compare values of any type.
fn row_bytes(conv: &RowConverter, columns: &[ArrayRef]) -> Result<Vec<Vec<u8>>> {
    let rows = conv.convert_columns(columns)?;
    Ok(rows.iter().map(|r| r.as_ref().to_vec()).collect())
}

/// The key values in their order: `keys` in its order, or the distinct non-null keys of
/// the input ascending; then the values of `extra_keys` that are not already key values,
/// ascending (without `keys`, all ascending together).
fn key_values(
    key: &ArrayRef,
    keys: Option<&[ScalarValue]>,
    extra: &[ScalarValue],
) -> Result<ArrayRef> {
    let conv = RowConverter::new(vec![SortField::new(key.data_type().clone())])?;
    let distinct = |a: &ArrayRef, skip: &HashSet<Vec<u8>>| -> Result<Vec<u32>> {
        let mut seen = skip.clone();
        let bytes = row_bytes(&conv, std::slice::from_ref(a))?;
        Ok((0..a.len())
            .filter(|&i| a.is_valid(i) && seen.insert(bytes[i].clone()))
            .map(|i| i as u32)
            .collect())
    };
    // Sorted in the order of the row format, which every key type has, structs too.
    let sorted = |a: ArrayRef| -> Result<ArrayRef> {
        let rows = conv.convert_columns(std::slice::from_ref(&a))?;
        let mut order: Vec<u32> = (0..a.len() as u32).collect();
        order.sort_by(|x, y| rows.row(*x as usize).cmp(&rows.row(*y as usize)));
        Ok(take(&a, &UInt32Array::from(order), None)?)
    };
    let to_array = |v: &[ScalarValue]| -> Result<ArrayRef> {
        if v.is_empty() {
            Ok(new_null_array(key.data_type(), 0))
        } else {
            ScalarValue::iter_to_array(v.iter().cloned())
        }
    };
    let extra = to_array(extra)?;
    match keys {
        Some(listed) => {
            let listed = to_array(listed)?;
            let listed_bytes: HashSet<_> = row_bytes(&conv, std::slice::from_ref(&listed))?
                .into_iter()
                .collect();
            let more = take(
                &extra,
                &UInt32Array::from(distinct(&extra, &listed_bytes)?),
                None,
            )?;
            Ok(concat(&[listed.as_ref(), sorted(more)?.as_ref()])?)
        }
        None => {
            let all = concat(&[key.as_ref(), extra.as_ref()])?;
            let idx = distinct(&all, &HashSet::new())?;
            sorted(take(&all, &UInt32Array::from(idx), None)?)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn impute_batch(
    input: &RecordBatch,
    key: usize,
    groups: &[usize],
    val: usize,
    fill: &Fill,
    keys: Option<&[ScalarValue]>,
    extra_keys: &[ScalarValue],
    out_schema: SchemaRef,
) -> Result<RecordBatch> {
    let n = input.num_rows();
    let key_col = input.column(key);
    let domain = key_values(key_col, keys, extra_keys)?;
    let key_conv = RowConverter::new(vec![SortField::new(key_col.data_type().clone())])?;
    let domain_index: HashMap<Vec<u8>, usize> =
        row_bytes(&key_conv, std::slice::from_ref(&domain))?
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, i))
            .collect();
    let row_key: Vec<Option<usize>> = row_bytes(&key_conv, std::slice::from_ref(key_col))?
        .iter()
        .enumerate()
        .map(|(i, b)| {
            key_col
                .is_valid(i)
                .then(|| domain_index.get(b).copied())
                .flatten()
        })
        .collect();

    // Groups in the order of their first row.
    let group_cols: Vec<ArrayRef> = groups.iter().map(|&i| input.column(i).clone()).collect();
    let row_group: Vec<usize>;
    let mut reps: Vec<usize> = Vec::new();
    if group_cols.is_empty() {
        row_group = vec![0; n];
        if n > 0 {
            reps.push(0);
        }
    } else {
        let fields = group_cols
            .iter()
            .map(|c| SortField::new(c.data_type().clone()))
            .collect();
        let conv = RowConverter::new(fields)?;
        let mut index: HashMap<Vec<u8>, usize> = HashMap::new();
        row_group = row_bytes(&conv, &group_cols)?
            .into_iter()
            .enumerate()
            .map(|(i, b)| {
                *index.entry(b).or_insert_with(|| {
                    reps.push(i);
                    reps.len() - 1
                })
            })
            .collect();
    }

    // The rows of each group at each key value.
    let mut cells: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for i in 0..n {
        if let Some(d) = row_key[i] {
            cells.entry((row_group[i], d)).or_default().push(i);
        }
    }

    // The added rows: each group, each missing key value, in key order.
    let mut added: Vec<(usize, usize)> = Vec::new();
    for g in 0..reps.len() {
        for d in 0..domain.len() {
            if !cells.contains_key(&(g, d)) {
                added.push((g, d));
            }
        }
    }
    let m = added.len();

    let val_col = input.column(val);
    let val_type = val_col.data_type().clone();
    let imputed: ArrayRef = match fill {
        Fill::Value(v) => v.to_array_of_size(m)?,
        Fill::Gap => new_null_array(&val_type, m),
        Fill::Stat { stat, frame } => {
            let measures = cast(val_col, &DataType::Float64)?;
            let measures = measures.as_primitive::<Float64Type>();
            let measure = |i: usize| measures.is_valid(i).then(|| measures.value(i));
            let values: Vec<Option<f64>> = match frame {
                // A frame of the whole group is no frame.
                None | Some((None, None)) => {
                    let mut by_group: Vec<Vec<f64>> = vec![Vec::new(); reps.len()];
                    for i in 0..n {
                        if let Some(v) = measure(i) {
                            by_group[row_group[i]].push(v);
                        }
                    }
                    let per_group: Vec<Option<f64>> =
                        by_group.iter_mut().map(|v| stat.of(v)).collect();
                    added.iter().map(|&(g, _)| per_group[g]).collect()
                }
                Some((lo, hi)) => {
                    // The positions of each group after imputation, in key order: the
                    // existing rows of each key value (in input order), or one added row.
                    let mut out = Vec::with_capacity(m);
                    for g in 0..reps.len() {
                        let mut slots: Vec<Option<usize>> = Vec::new();
                        for d in 0..domain.len() {
                            match cells.get(&(g, d)) {
                                Some(rows) => slots.extend(rows.iter().map(|&r| Some(r))),
                                None => slots.push(None),
                            }
                        }
                        let len = slots.len() as i64;
                        for (p, slot) in slots.iter().enumerate() {
                            if slot.is_some() {
                                continue;
                            }
                            let p = p as i64;
                            let start = lo.map_or(0, |lo| p.saturating_add(lo).max(0));
                            let end = hi.map_or(len - 1, |hi| p.saturating_add(hi).min(len - 1));
                            let mut window: Vec<f64> = (start..=end.max(start - 1))
                                .filter_map(|q| slots[q as usize].and_then(measure))
                                .collect();
                            out.push(stat.of(&mut window));
                        }
                    }
                    out
                }
            };
            cast(
                &(Arc::new(Float64Array::from(values)) as ArrayRef),
                &val_type,
            )?
        }
    };

    let added_keys = UInt32Array::from(added.iter().map(|&(_, d)| d as u32).collect::<Vec<_>>());
    let added_reps = UInt32Array::from(
        added
            .iter()
            .map(|&(g, _)| reps[g] as u32)
            .collect::<Vec<_>>(),
    );
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(out_schema.fields().len());
    for (i, col) in input.columns().iter().enumerate() {
        let tail: ArrayRef = if i == key {
            take(&domain, &added_keys, None)?
        } else if groups.contains(&i) {
            take(col, &added_reps, None)?
        } else if i == val {
            imputed.clone()
        } else {
            new_null_array(col.data_type(), m)
        };
        columns.push(concat(&[col.as_ref(), tail.as_ref()])?);
    }
    if out_schema.fields().len() > input.num_columns() {
        let flags: Vec<bool> = (0..n + m).map(|i| i >= n).collect();
        columns.push(Arc::new(BooleanArray::from(flags)));
    }
    Ok(RecordBatch::try_new_with_options(
        out_schema,
        columns,
        &RecordBatchOptions::new().with_row_count(Some(n + m)),
    )?)
}

// ---------------------------------------------------------------------------------------
// Physical planning
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct ImputeExtensionPlanner;

#[async_trait]
impl ExtensionPlanner for ImputeExtensionPlanner {
    async fn plan_extension(
        &self,
        _planner: &dyn PhysicalPlanner,
        node: &dyn UserDefinedLogicalNode,
        logical_inputs: &[&LogicalPlan],
        physical_inputs: &[Arc<dyn ExecutionPlan>],
        _session: &dyn Session,
        _ctx: &PhysicalPlanningContext,
    ) -> Result<Option<Arc<dyn ExecutionPlan>>> {
        let Some(n) = node.as_any().downcast_ref::<ImputeNode>() else {
            return Ok(None);
        };
        let schema = logical_inputs[0].schema();
        let key = schema.index_of_column(&n.key)?;
        let groups = n
            .groupby
            .iter()
            .map(|c| schema.index_of_column(c))
            .collect::<Result<Vec<_>>>()?;
        let val = schema.index_of_column(&n.val)?;
        Ok(Some(Arc::new(ImputeExec::new(
            physical_inputs[0].clone(),
            key,
            groups,
            val,
            n.fill.clone(),
            n.keys.clone(),
            n.extra_keys.clone(),
            Arc::new(n.schema.as_arrow().clone()),
        ))))
    }
}

/// The default query planner, with the extension planner of `impute`.
#[derive(Debug)]
pub struct TransformsQueryPlanner;

#[async_trait]
impl QueryPlanner for TransformsQueryPlanner {
    async fn create_physical_plan(
        &self,
        logical_plan: &LogicalPlan,
        session: &dyn Session,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        DefaultPhysicalPlanner::with_extension_planners(vec![Arc::new(ImputeExtensionPlanner)])
            .create_physical_plan(logical_plan, session)
            .await
    }
}
