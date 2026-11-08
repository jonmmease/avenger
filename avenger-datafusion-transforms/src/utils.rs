//! What the transforms share: the fields of a struct of a start and an end, the constant
//! options of a call, the window nodes and partition values of the optimizer rules, the
//! windows of SQL that runs within a scope, and [`StructGroupSplit`], the rule that groups an
//! aggregate by the fields of a struct key.

use std::collections::BTreeMap;
use std::sync::Arc;

use datafusion::arrow::array::{Array, AsArray, StructArray};
use datafusion::arrow::buffer::NullBuffer;
use datafusion::arrow::datatypes::{DataType, Field, FieldRef, Fields};
use datafusion::common::tree_node::{Transformed, TreeNode};
use datafusion::common::{
    Column, JoinType, NullEquality, Result, ScalarValue, Spans, plan_datafusion_err, plan_err,
};
use datafusion::functions::core::expr_fn::get_field;
use datafusion::logical_expr::expr::{WindowFunction, WindowFunctionParams};
use datafusion::logical_expr::{
    Aggregate, ColumnarValue, Expr, ExprSchemable, LogicalPlan, LogicalPlanBuilder,
    ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Subquery, Volatility,
    Window, WindowFunctionDefinition, WindowUDFImpl, lit, when,
};
use datafusion::optimizer::{ApplyOrder, OptimizerConfig, OptimizerRule};
use datafusion::physical_expr::PhysicalExpr;
use datafusion::physical_expr::expressions::Literal;
use datafusion::sql::sqlparser::ast::{Expr as SqlExpr, WindowSpec, WindowType};

// ---------------------------------------------------------------------------------------
// Structs and options
// ---------------------------------------------------------------------------------------

/// The fields of the result of `bin` and `stack`: a `start` and an `end`.
pub(crate) fn interval_fields() -> Fields {
    Fields::from(vec![
        Field::new("start", DataType::Float64, false),
        Field::new("end", DataType::Float64, false),
    ])
}

/// The constant values of the expressions of a call's options, in a logical plan.
pub(crate) fn literals(name: &str, exprs: &[Expr]) -> Result<Vec<ScalarValue>> {
    exprs
        .iter()
        .map(|e| match e {
            Expr::Literal(v, _) => Ok(v.clone()),
            _ => plan_err!("{name} options must be constants, not {e}"),
        })
        .collect()
}

/// The constant values of the expressions of a call's options, in a physical plan, as a
/// window evaluator or an aggregate gets them.
pub(crate) fn physical_literals(
    name: &str,
    exprs: &[Arc<dyn PhysicalExpr>],
) -> Result<Vec<ScalarValue>> {
    exprs
        .iter()
        .map(|e| {
            e.downcast_ref::<Literal>()
                .map(|l| l.value().clone())
                .ok_or_else(|| plan_datafusion_err!("{name} options must be constants, not {e}"))
        })
        .collect()
}

/// The constant values of a scalar function's options, as its invocation gets them.
pub(crate) fn scalar_args(name: &str, args: &[ColumnarValue]) -> Result<Vec<ScalarValue>> {
    args.iter()
        .map(|a| match a {
            ColumnarValue::Scalar(v) => Ok(v.clone()),
            ColumnarValue::Array(_) => plan_err!("{name} options must be constants"),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// Window nodes, for the rules
// ---------------------------------------------------------------------------------------

/// The call of a window expression. The simplifier keeps the name of a call whose options
/// it folds (`[0, 100]` becomes a list constant) by wrapping the call in an alias.
pub(crate) fn window_call(expr: &Expr) -> Option<&WindowFunction> {
    let expr = match expr {
        Expr::Alias(a) => a.expr.as_ref(),
        e => e,
    };
    match expr {
        Expr::WindowFunction(wf) => Some(wf.as_ref()),
        _ => None,
    }
}

/// Whether a window expression calls the window function `T`.
pub(crate) fn is_call<T: WindowUDFImpl>(expr: &Expr) -> bool {
    window_call(expr).is_some_and(
        |wf| matches!(&wf.fun, WindowFunctionDefinition::WindowUDF(u) if u.inner().is::<T>()),
    )
}

/// Checks the OVER clause of a function of whole partitions: no ORDER BY, no frame that is
/// not the whole partition, and no DISTINCT or null treatment, which it would ignore.
pub(crate) fn whole_partition(name: &str, params: &WindowFunctionParams) -> Result<()> {
    if !params.order_by.is_empty() {
        return plan_err!("the window of {name} cannot have an ORDER BY");
    }
    let frame = &params.window_frame;
    if !(frame.start_bound.is_unbounded() && frame.end_bound.is_unbounded()) {
        return plan_err!("the window of {name} is its whole partition, so it cannot have a frame");
    }
    if params.distinct || params.null_treatment.is_some() || params.filter.is_some() {
        return plan_err!("{name} takes no DISTINCT, IGNORE NULLS, or FILTER");
    }
    Ok(())
}

/// Whether a plan computes anything volatile, such as `random()`. A rule that reads the
/// input of a window two times would read different values.
pub(crate) fn is_volatile(plan: &LogicalPlan) -> Result<bool> {
    plan.exists(|p| Ok(p.expressions().iter().any(Expr::is_volatile)))
}

/// A window node whose calls a rule takes only some of: a window node of those calls over
/// the input, under a window node of the others.
#[derive(Debug, Clone)]
pub(crate) struct Split {
    /// The window node of the calls that the rule takes.
    pub lower: LogicalPlan,
    others: Vec<Expr>,
    schema: datafusion::common::DFSchemaRef,
}

/// Splits a window node into the calls that `take` takes and the others, or None when it
/// takes all or none of them.
pub(crate) fn split_window(
    window: &Window,
    take: impl Fn(usize, &Expr) -> bool,
) -> Result<Option<Split>> {
    let (taken, others): (Vec<_>, Vec<_>) = window
        .window_expr
        .iter()
        .enumerate()
        .partition(|(i, e)| take(*i, e));
    if taken.is_empty() || others.is_empty() {
        return Ok(None);
    }
    let taken = taken.into_iter().map(|(_, e)| e.clone()).collect();
    let lower = LogicalPlan::Window(Window::try_new(taken, window.input.clone())?);
    Ok(Some(Split {
        lower,
        others: others.into_iter().map(|(_, e)| e.clone()).collect(),
        schema: window.schema.clone(),
    }))
}

impl Split {
    /// The window node of the other calls over `lower`, which has the schema of the lower
    /// window node, rewritten by `upper`, then the columns in the order of the window node
    /// that was split.
    pub fn rebuild(
        self,
        lower: LogicalPlan,
        upper: impl FnOnce(LogicalPlan) -> Result<LogicalPlan>,
    ) -> Result<LogicalPlan> {
        let upper = upper(LogicalPlan::Window(Window::try_new(
            self.others,
            Arc::new(lower),
        )?))?;
        let columns: Vec<Expr> = self
            .schema
            .iter()
            .map(|(q, f)| Expr::Column(Column::new(q.cloned(), f.name())))
            .collect();
        LogicalPlanBuilder::from(upper).project(columns)?.build()
    }
}

/// The calls of a window node that a rewrite of a function of whole partitions takes: every
/// call is a call of `T` over one `PARTITION BY` of columns, and the node's input computes
/// nothing volatile, which the rewrite would read two times. None leaves the node to the
/// window evaluator; a call with ORDER BY, a frame, DISTINCT, or FILTER is an error.
pub(crate) struct PartitionCalls<'a> {
    pub calls: Vec<&'a WindowFunction>,
    /// The `PARTITION BY` columns.
    pub keys: Vec<Column>,
}

pub(crate) fn partition_calls<'a, T: WindowUDFImpl>(
    window: &'a Window,
    name: &str,
) -> Result<Option<PartitionCalls<'a>>> {
    let mut calls: Vec<&WindowFunction> = Vec::new();
    for expr in &window.window_expr {
        let Some(wf) = window_call(expr).filter(|_| is_call::<T>(expr)) else {
            return Ok(None);
        };
        whole_partition(name, &wf.params)?;
        if wf.params.args.is_empty() {
            return Ok(None);
        }
        calls.push(wf);
    }
    if is_volatile(&window.input)? || calls.iter().any(|c| c.params.args[0].is_volatile()) {
        return Ok(None);
    }
    let partition_by = &calls[0].params.partition_by;
    if calls.iter().any(|c| &c.params.partition_by != partition_by) {
        return Ok(None);
    }
    let keys = partition_by
        .iter()
        .map(|e| match e {
            Expr::Column(c) => Some(c.clone()),
            _ => None,
        })
        .collect::<Option<Vec<Column>>>();
    Ok(keys.map(|keys| PartitionCalls { calls, keys }))
}

/// A value of each window partition: aggregates of the partition's rows, with unique
/// aliases, and an expression of the aggregates by those aliases.
pub(crate) struct PartitionValue {
    pub aggregates: Vec<Expr>,
    pub value: Expr,
}

/// The input with each value of [`PartitionValue`] for each row: without keys, each value is a
/// scalar subquery of the input; with keys, one aggregate of the input by the keys, joined back
/// to the rows on the keys, where NULL keys match, as PARTITION BY puts them in one partition.
/// `name` names the aliases. Returns the builder of the input or the join, and the expression
/// of each value.
pub(crate) fn partition_values(
    input: LogicalPlan,
    keys: &[Column],
    values: Vec<PartitionValue>,
    name: &str,
) -> Result<(LogicalPlanBuilder, Vec<Expr>)> {
    if values.is_empty() {
        return Ok((LogicalPlanBuilder::from(input), vec![]));
    }
    if keys.is_empty() {
        let mut exprs = Vec::with_capacity(values.len());
        for v in values {
            let subquery = LogicalPlanBuilder::from(input.clone())
                .aggregate(Vec::<Expr>::new(), v.aggregates)?
                .project(vec![v.value])?
                .build()?;
            exprs.push(Expr::ScalarSubquery(Subquery {
                subquery: Arc::new(subquery),
                outer_ref_columns: vec![],
                spans: Spans::new(),
            }));
        }
        return Ok((LogicalPlanBuilder::from(input), exprs));
    }
    let partitions = format!("__{name}_partitions");
    let key = |k: usize| format!("__{name}_key_{k}");
    let group: Vec<Expr> = keys
        .iter()
        .enumerate()
        .map(|(k, c)| Expr::Column(c.clone()).alias(key(k)))
        .collect();
    let mut projection: Vec<Expr> = (0..keys.len())
        .map(|k| Expr::Column(Column::from_name(key(k))))
        .collect();
    let mut aggregates = Vec::new();
    let mut exprs = Vec::with_capacity(values.len());
    for (i, v) in values.into_iter().enumerate() {
        aggregates.extend(v.aggregates);
        let alias = format!("__{name}_value_{i}");
        projection.push(v.value.alias(&alias));
        exprs.push(Expr::Column(Column::new(Some(partitions.as_str()), alias)));
    }
    let per_partition = LogicalPlanBuilder::from(input.clone())
        .aggregate(group, aggregates)?
        .project(projection)?
        .alias(partitions.as_str())?
        .build()?;
    let right: Vec<Column> = (0..keys.len())
        .map(|k| Column::new(Some(partitions.as_str()), key(k)))
        .collect();
    let builder = LogicalPlanBuilder::from(input).join_detailed(
        per_partition,
        JoinType::Inner,
        (keys.to_vec(), right),
        None,
        NullEquality::NullEqualsNull,
    )?;
    Ok((builder, exprs))
}

// ---------------------------------------------------------------------------------------
// Rule 2: GROUP BY struct -> GROUP BY its fields, then the struct again
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct StructGroupSplit;

/// A UDF that makes the struct of a field from one array for each of its fields. The struct
/// has exactly the type and the metadata of the field: unlike `named_struct`, its fields keep
/// their nullability. If the field can be NULL, the first argument is true where the struct
/// is NULL.
pub(crate) fn struct_udf(field: &Field) -> ScalarUDF {
    let DataType::Struct(fields) = field.data_type() else {
        unreachable!("a struct field")
    };
    let nullable = field.is_nullable();
    ScalarUDF::new_from_impl(MakeStruct {
        fields: fields.clone(),
        nullable,
        metadata: field.metadata().clone().into_iter().collect(),
        signature: Signature::any(fields.len() + usize::from(nullable), Volatility::Immutable),
    })
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct MakeStruct {
    fields: Fields,
    nullable: bool,
    metadata: BTreeMap<String, String>,
    signature: Signature,
}

impl ScalarUDFImpl for MakeStruct {
    fn name(&self) -> &str {
        if self.nullable {
            "__make_nullable_struct"
        } else {
            "__make_struct"
        }
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Struct(self.fields.clone()))
    }
    fn return_field_from_args(&self, _: ReturnFieldArgs) -> Result<FieldRef> {
        Ok(Arc::new(
            Field::new(
                self.name(),
                DataType::Struct(self.fields.clone()),
                self.nullable,
            )
            .with_metadata(self.metadata.clone().into_iter().collect()),
        ))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let n = args.number_rows;
        let mut arrays = args
            .args
            .iter()
            .map(|a| a.to_array(n))
            .collect::<Result<Vec<_>>>()?;
        let nulls = if self.nullable {
            let is_null = arrays.remove(0);
            let is_null = is_null.as_boolean();
            Some(NullBuffer::from_iter(
                (0..n).map(|i| !(is_null.is_valid(i) && is_null.value(i))),
            ))
        } else {
            None
        };
        Ok(ColumnarValue::Array(Arc::new(StructArray::try_new(
            self.fields.clone(),
            arrays,
            nulls.filter(|n| n.null_count() > 0),
        )?)))
    }
}

/// Whether the fields of a struct key can be group keys of their own: at least one, each with
/// a name that is not empty and no other field has, and none a struct.
fn splittable(fields: &Fields) -> bool {
    let mut names = std::collections::HashSet::new();
    !fields.is_empty()
        && fields.iter().all(|f| {
            !f.name().is_empty()
                && names.insert(f.name())
                && !matches!(f.data_type(), DataType::Struct(_))
        })
}

impl OptimizerRule for StructGroupSplit {
    fn name(&self) -> &str {
        "struct_group_split"
    }

    fn apply_order(&self) -> Option<ApplyOrder> {
        Some(ApplyOrder::BottomUp)
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        _config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::Aggregate(agg) = &plan else {
            return Ok(Transformed::no(plan));
        };
        if agg
            .group_expr
            .iter()
            .any(|e| matches!(e, Expr::GroupingSet(_)))
        {
            return Ok(Transformed::no(plan));
        }
        let input_schema = agg.input.schema();
        let mut group = Vec::new();
        // For each group key: the fields that replace it and whether it can be NULL, or None
        // if it stays.
        let mut split: Vec<Option<(Fields, bool)>> = Vec::new();
        for (i, e) in agg.group_expr.iter().enumerate() {
            let (_, field) = e.to_field(input_schema)?;
            match field.data_type().clone() {
                // The rule reads the fields by name, and does not split nested structs. A
                // NULL struct and a struct of NULL fields are different keys, so a key that
                // can be NULL also groups by whether it is NULL, and its fields are NULL
                // where it is.
                DataType::Struct(fields) if splittable(&fields) => {
                    let inner = e.clone().unalias();
                    let nullable = field.is_nullable();
                    if nullable {
                        group.push(inner.clone().is_null().alias(format!("__group_{i}_null")));
                    }
                    for (j, f) in fields.iter().enumerate() {
                        let value = get_field(inner.clone(), f.name().as_str());
                        let value = if nullable {
                            when(
                                inner.clone().is_null(),
                                lit(ScalarValue::try_from(f.data_type())?),
                            )
                            .otherwise(value)?
                        } else {
                            value
                        };
                        group.push(value.alias(format!("__group_{i}_{j}")));
                    }
                    split.push(Some((fields, nullable)));
                }
                _ => {
                    group.push(e.clone());
                    split.push(None);
                }
            }
        }
        if split.iter().all(Option::is_none) {
            return Ok(Transformed::no(plan));
        }

        let n_group = group.len();
        let new_agg = LogicalPlan::Aggregate(Aggregate::try_new(
            agg.input.clone(),
            group,
            agg.aggr_expr.clone(),
        )?);
        let new_schema = new_agg.schema().clone();
        let column = |k: usize| Expr::Column(Column::from(new_schema.qualified_field(k)));

        // The projection gives each output column the name, qualifier and type it had.
        let mut exprs = Vec::new();
        let mut k = 0;
        for (i, fields) in split.iter().enumerate() {
            let (q, f) = agg.schema.qualified_field(i);
            let expr = match fields {
                None => {
                    k += 1;
                    column(k - 1)
                }
                Some((fields, nullable)) => {
                    let n = fields.len() + usize::from(*nullable);
                    let args = (k..k + n).map(column).collect();
                    k += n;
                    struct_udf(f).call(args)
                }
            };
            // An alias to the same name confuses push_down_leaf_projections (see below).
            let same =
                matches!(&expr, Expr::Column(c) if c.relation.as_ref() == q && &c.name == f.name());
            exprs.push(if same {
                expr
            } else {
                expr.alias_qualified(q.cloned(), f.name())
            });
        }
        for a in 0..agg.aggr_expr.len() {
            let (q, f) = agg.schema.qualified_field(split.len() + a);
            let (nq, nf) = new_schema.qualified_field(n_group + a);
            // An alias to the same name confuses DataFusion's push_down_leaf_projections,
            // which then keeps both the column and the alias, two fields of one name.
            if nq == q && nf.name() == f.name() {
                exprs.push(column(n_group + a));
            } else {
                exprs.push(column(n_group + a).alias_qualified(q.cloned(), f.name()));
            }
        }
        Ok(Transformed::yes(
            LogicalPlanBuilder::from(new_agg).project(exprs)?.build()?,
        ))
    }
}

// ---------------------------------------------------------------------------------------
// The windows of a scope
// ---------------------------------------------------------------------------------------

/// The windows of SQL that runs within a scope, such as the categories of one window
/// partition of `lump`. Every window runs within the scope's partition, and a window can
/// build on a named window of the scope, as SQL's named windows do: `OVER name` is the named
/// window, and `OVER (name <frame>)` adds a frame to it.
pub(crate) struct WindowScope<'a> {
    /// What the SQL is, for errors, such as "the 'keep' of lump".
    pub what: &'a str,
    /// The partition that every window gets.
    pub partition: Vec<SqlExpr>,
    /// The named windows: an order each, without a partition or a frame.
    pub named: Vec<(&'a str, WindowSpec)>,
}

impl WindowScope<'_> {
    /// The window of `over` within the scope. A `PARTITION BY`, an unknown name, an
    /// `ORDER BY` added to a named window, and a frame without an order are errors: the rows
    /// of a scope have no order of their own.
    pub fn resolve(&self, over: &WindowType) -> std::result::Result<WindowType, String> {
        let (name, written) = match over {
            WindowType::NamedWindow(name) => (Some(name), None),
            WindowType::WindowSpec(spec) => (spec.window_name.as_ref(), Some(spec)),
        };
        let names = || {
            self.named
                .iter()
                .map(|(n, _)| format!("`{n}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if written.is_some_and(|w| !w.partition_by.is_empty()) {
            return Err(format!(
                "a window in {} already runs within its partition; remove its PARTITION BY",
                self.what
            ));
        }
        let base = match name {
            None => None,
            Some(name) => Some(
                self.named
                    .iter()
                    .find(|(n, _)| name.value.eq_ignore_ascii_case(n))
                    .map(|(_, w)| w)
                    .ok_or_else(|| {
                        format!(
                            "unknown window `{name}` in {}; its windows are {}",
                            self.what,
                            names()
                        )
                    })?,
            ),
        };
        let written_order = written.map_or(&[][..], |w| &w.order_by[..]);
        let order_by = match base {
            Some(_) if !written_order.is_empty() => {
                return Err(format!(
                    "the window `{}` in {} has an ORDER BY already; add only a frame to it, or write a window of its own",
                    name.expect("a base has a name"),
                    self.what
                ));
            }
            Some(base) => base.order_by.clone(),
            None => written_order.to_vec(),
        };
        let window_frame = written.and_then(|w| w.window_frame.clone());
        if window_frame.is_some() && order_by.is_empty() {
            return Err(format!(
                "a window frame in {} needs an order, from an ORDER BY or a window such as {}",
                self.what,
                names()
            ));
        }
        Ok(WindowType::WindowSpec(WindowSpec {
            window_name: None,
            partition_by: self.partition.clone(),
            order_by,
            window_frame,
        }))
    }
}
