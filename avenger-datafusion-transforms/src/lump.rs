//! `lump`: keeps the top categories of each partition of the rows, or the categories that a
//! condition keeps, and lumps the rest into an Other group.
//!
//! [`plan`] adds the lump of each row to its input, from the category, the partition, and the
//! planned expressions of the options, which a SQL front end plans in the scopes that they read:
//!
//! - `weight`: the weight of each category, an aggregate of its rows, `COUNT(*)` by default. It
//!   orders the window `ranked`, [`ranked`]: the categories in weight order, `ORDER BY <weight>
//!   DESC NULLS LAST, <category> ASC NULLS LAST`.
//! - `keep`: [`Keep::First`] keeps the first `n` categories of `ranked`, and
//!   [`Keep::Condition`] the categories for which a condition holds. The condition is
//!   evaluated once for each category, over its rows: like a `HAVING` clause it can compare
//!   aggregates of the category's rows (`SUM(tons) > 1000`), and like `QUALIFY` it can use
//!   windows over the categories (`COUNT(*) >= 0.05 * SUM(COUNT(*)) OVER ()`). Every window in
//!   it runs over the categories of the lump's partition, apart from the NULL category, so it
//!   takes no `PARTITION BY` of its own. A NULL condition lumps the category.
//! - `other`: the label of the Other group, `'Other'` by default.
//! - `null_category`: the NULL category, which `keep` doesn't decide, [`Nulls`].
//!
//! The result is `struct<value, other, weight, category_count>`: for a kept category, the
//! category, its weight, and 1; for the Other group, the label, the weight of all rows of the
//! lumped categories, and the number of lumped categories of the partition, for a label such as
//! "Other (12)". The weight is a `DOUBLE`. A kept NULL category has NULL in both `value` and
//! `other`, so `other` tells it from the Other group, and the NULL category counts in the
//! weight and the `category_count` of the Other group only with [`Nulls::Other`].
//!
//! The plan groups the rows by the partition and the category, decides each category, and
//! joins the decisions to the rows, so it reads the input more than once. An aggregate grouped
//! by a lump, and by the lump's partition, reads it once through [`fuse`], which the analyzer
//! rule [`LumpAggregateFusion`] applies: the categories hold the states of the aggregates of
//! their rows, and each value of the lump merges them, for the aggregates that merge from
//! states (`COUNT`, `SUM`, `MIN`, `MAX`, `AVG`, and the variances and standard deviations).
//!
//! A front end that plans a lump as an expression of a projection, beside other windows, writes
//! it as [`window`], a placeholder window function of the call, and plans the windows of the
//! projection through [`plan_windows`], which plans each lump onto the input.

use std::any::Any;
use std::sync::Arc;

use avenger_datafusion_aggregate_state::{Family, merge, state};
use datafusion::arrow::datatypes::{DataType, Field, FieldRef, Fields};
use datafusion::common::config::ConfigOptions;
use datafusion::common::{Column, NullEquality, Result, ScalarValue, plan_err};
use datafusion::functions_aggregate::count::count_udaf;
use datafusion::functions_window::row_number::row_number_udwf;
use datafusion::logical_expr::expr::{AggregateFunction, WindowFunction};
use datafusion::logical_expr::function::{PartitionEvaluatorArgs, WindowUDFFieldArgs};
use datafusion::logical_expr::utils::{find_aggregate_exprs, find_window_exprs};
use datafusion::logical_expr::{
    Cast, ColumnarValue, Expr, ExprSchemable, JoinType, LogicalPlan, LogicalPlanBuilder,
    PartitionEvaluator, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature,
    SortExpr, Volatility, WindowFrame, WindowFunctionDefinition, WindowUDF, WindowUDFImpl, when,
};
use datafusion::optimizer::AnalyzerRule;

/// What a lump does with the NULL category, which `keep` doesn't decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Nulls {
    /// The NULL category is a category of its own, in each partition that has NULL rows.
    #[default]
    Keep,
    /// The NULL category is in the Other group.
    Other,
    /// The rows of the NULL category have a NULL lump, in no category.
    Drop,
}

impl Nulls {
    /// The option's value of the name `name`: `'keep'`, `'other'` or `'drop'`.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "keep" => Some(Nulls::Keep),
            "other" => Some(Nulls::Other),
            "drop" => Some(Nulls::Drop),
            _ => None,
        }
    }

    /// The decision of the NULL category: kept, lumped into the Other group, or neither.
    fn decision(self) -> Expr {
        let value = match self {
            Nulls::Keep => Some(true),
            Nulls::Other => Some(false),
            Nulls::Drop => None,
        };
        Expr::Literal(ScalarValue::Boolean(value), None)
    }
}

/// Which categories a lump keeps.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Keep {
    /// The first `n` categories of `ranked`.
    First(i64),
    /// The categories for which the condition holds, an expression over the input of aggregates
    /// of a category's rows and windows over the categories.
    Condition(Expr),
}

/// A call of `lump` over an input: the category and the partition, expressions over the
/// input, and the options.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LumpCall {
    pub category: Expr,
    pub partition: Vec<Expr>,
    /// The aggregate of a category's rows that weighs it, or `COUNT(*)`.
    pub weight: Option<Expr>,
    pub keep: Keep,
    pub other: String,
    pub null_category: Nulls,
}

/// The fields of the struct of a lump of a category of the type `category`.
pub fn lump_fields(category: &DataType) -> Fields {
    Fields::from(vec![
        Field::new("value", category.clone(), true),
        Field::new("other", DataType::Utf8, true),
        Field::new("weight", DataType::Float64, true),
        Field::new("category_count", DataType::Int64, false),
    ])
}

/// `COUNT(*)`, the default weight.
pub fn count_star() -> Expr {
    Expr::AggregateFunction(AggregateFunction::new_udf(
        count_udaf(),
        vec![Expr::Literal(ScalarValue::Int64(Some(1)), None)],
        false,
        None,
        Vec::new(),
        None,
    ))
}

/// The order of the window `ranked` of a lump of the weight `weight` and the category
/// `category`: the categories in weight order, the heaviest first, and by category on a tie.
pub fn ranked(weight: &Expr, category: &Expr) -> Vec<SortExpr> {
    vec![
        SortExpr::new(weight.clone(), false, false),
        SortExpr::new(category.clone(), true, false),
    ]
}

/// The names of the columns that the plan of a lump adds, and removes again.
const CAT: &str = "__lump_cat";
const KEY: &str = "__lump_key_";
const D_CAT: &str = "__lump_d_cat";
const D_KEY: &str = "__lump_d_key_";
const WEIGHT: &str = "__lump_weight";
const KEEP: &str = "__lump_keep";
const OTHER: &str = "__lump_other";
const LUMPED: &str = "__lump_lumped";

fn col(name: &str) -> Expr {
    Expr::Column(Column::from_name(name))
}

fn cast(expr: Expr, ty: DataType) -> Expr {
    Expr::Cast(Cast::new(Box::new(expr), ty))
}

/// `expr` with each of `bases`, aggregates or windows, written as its column of the node above
/// them.
fn rebase(expr: Expr, bases: &[Expr]) -> Result<Expr> {
    use datafusion::common::tree_node::{Transformed, TreeNode};
    expr.transform_down(|e| match bases.contains(&e) {
        true => Ok(Transformed::yes(Expr::Column(Column::from_name(
            e.schema_name().to_string(),
        )))),
        false => Ok(Transformed::no(e)),
    })
    .map(|t| t.data)
}

/// The stages of a lump's plan that [`plan`] and [`fuse`] share.
struct Decided {
    /// The rows of the input, with the category and the partition keys as columns.
    rows: LogicalPlan,
    /// One row for each category of each partition: its keys, its weight, its decision, and the
    /// extra aggregates of its rows.
    decided: LogicalPlan,
    keys: Vec<String>,
    d_keys: Vec<String>,
    category_type: DataType,
    weight: Expr,
}

/// The rows of `input` and the decision of each category by `call`, with `extra`, aggregates of
/// a category's rows each with an alias, as columns of the categories.
fn decide(input: LogicalPlan, call: &LumpCall, extra: Vec<Expr>) -> Result<Decided> {
    let LumpCall {
        category,
        partition,
        weight,
        keep,
        other: _,
        null_category,
    } = call.clone();
    let category_type = category.get_type(input.schema())?;
    let weight = weight.unwrap_or_else(count_star);
    if !find_window_exprs([&weight]).is_empty() {
        return plan_err!("the weight of lump is the weight of one category, so it has no window");
    }
    if find_aggregate_exprs([&weight]).is_empty() {
        return plan_err!("the weight of lump is an aggregate of the rows of a category");
    }
    let input_columns: Vec<Column> = input.schema().columns();
    let extra_names: Vec<String> = (extra.iter())
        .map(|e| e.schema_name().to_string())
        .collect();
    let keys: Vec<String> = (0..partition.len()).map(|i| format!("{KEY}{i}")).collect();
    let d_keys: Vec<String> = (0..partition.len())
        .map(|i| format!("{D_KEY}{i}"))
        .collect();

    // The rows, with the category and the partition keys as columns.
    let mut exprs: Vec<Expr> = input_columns.iter().cloned().map(Expr::Column).collect();
    exprs.push(category.clone().alias(CAT));
    for (expr, key) in partition.iter().zip(&keys) {
        exprs.push(expr.clone().alias(key));
    }
    let rows = LogicalPlanBuilder::from(input).project(exprs)?.build()?;

    // One row for each category of each partition, with the aggregates of the weight and of
    // the condition.
    let keep = match keep {
        Keep::Condition(condition) => condition,
        Keep::First(n) => {
            let mut window = WindowFunction::new(
                WindowFunctionDefinition::WindowUDF(row_number_udwf()),
                Vec::new(),
            );
            window.params.order_by = ranked(&weight, &category);
            Expr::from(window).lt_eq(Expr::Literal(ScalarValue::Int64(Some(n)), None))
        }
    };
    let aggregates = find_aggregate_exprs([&weight, &keep]);
    let mut groups: Vec<Expr> = keys.iter().map(|k| col(k)).collect();
    groups.push(col(CAT));
    let grouped = LogicalPlanBuilder::from(rows.clone())
        .aggregate(groups, aggregates.iter().cloned().chain(extra))?
        .build()?;
    // The category of the grouped rows is its group column, so the expressions read it there.
    let category_of = |expr: Expr| -> Result<Expr> {
        use datafusion::common::tree_node::{Transformed, TreeNode};
        let expr = rebase(expr, &aggregates)?;
        expr.transform_down(|e| match e == category {
            true => Ok(Transformed::yes(col(CAT))),
            false => Ok(Transformed::no(e)),
        })
        .map(|t| t.data)
    };
    let weight_col = category_of(weight.clone())?;
    let keep = category_of(keep)?;
    // The windows of the condition run over the categories of the partition, apart from the
    // NULL category.
    let windows: Vec<Expr> = find_window_exprs([&keep]);
    let mut scoped = Vec::new();
    for window in &windows {
        let Expr::WindowFunction(function) = window else {
            unreachable!("a window function");
        };
        let mut function = function.as_ref().clone();
        if !function.params.partition_by.is_empty() {
            return plan_err!(
                "a window in the keep of lump runs over the categories of its partition, so it takes no PARTITION BY"
            );
        }
        function.params.partition_by = (keys.iter().map(|k| col(k)))
            .chain([col(CAT).is_null()])
            .collect();
        if function.params.order_by.is_empty() {
            function.params.window_frame = WindowFrame::new(None);
        }
        scoped.push(Expr::from(function));
    }
    let keep = {
        use datafusion::common::tree_node::{Transformed, TreeNode};
        keep.transform_down(|e| match windows.iter().position(|w| *w == e) {
            Some(i) => Ok(Transformed::yes(Expr::Column(Column::from_name(
                scoped[i].schema_name().to_string(),
            )))),
            None => Ok(Transformed::no(e)),
        })
        .map(|t| t.data)?
    };
    let windowed = match scoped.is_empty() {
        true => grouped,
        false => LogicalPlanBuilder::window_plan(grouped, scoped)?,
    };
    if !matches!(
        keep.get_type(windowed.schema())?,
        DataType::Boolean | DataType::Null
    ) {
        return plan_err!("the keep of lump is a condition, not a value of another type");
    }
    // The decision of each category: kept, lumped, or for a dropped NULL category, neither.
    let decision = when(col(CAT).is_null(), null_category.decision()).otherwise(
        datafusion::functions::core::expr_fn::coalesce(vec![
            keep,
            Expr::Literal(ScalarValue::Boolean(Some(false)), None),
        ]),
    )?;
    let mut exprs: Vec<Expr> = (keys.iter().zip(&d_keys))
        .map(|(k, d)| col(k).alias(d))
        .collect();
    exprs.push(col(CAT).alias(D_CAT));
    exprs.push(cast(weight_col, DataType::Float64).alias(WEIGHT));
    exprs.push(decision.alias(KEEP));
    exprs.extend(extra_names.iter().map(|name| col(name)));
    let decided = LogicalPlanBuilder::from(windowed).project(exprs)?.build()?;
    Ok(Decided {
        rows,
        decided,
        keys,
        d_keys,
        category_type,
        weight,
    })
}

/// The lump struct of a category, the column `category`, from the columns of its decision, its
/// weight, the Other weight and the number of lumped categories.
fn lump_struct(category: &str, category_type: &DataType, other: String) -> Result<Expr> {
    let kept = col(KEEP);
    let lumped = col(KEEP).is_false();
    let parts = vec![
        // The struct is NULL where the rows have no lump.
        col(KEEP).is_null(),
        cast(
            when(kept.clone().is_true(), col(category)).end()?,
            category_type.clone(),
        ),
        when(
            lumped.clone(),
            Expr::Literal(ScalarValue::Utf8(Some(other)), None),
        )
        .end()?,
        when(kept.clone().is_true(), col(WEIGHT))
            .when(lumped.clone(), col(OTHER))
            .end()?,
        when(
            kept.is_true(),
            Expr::Literal(ScalarValue::Int64(Some(1)), None),
        )
        .when(lumped, col(LUMPED))
        .end()?,
    ];
    let field = Field::new("lump", DataType::Struct(lump_fields(category_type)), true);
    Ok(crate::utils::struct_udf(&field).call(parts))
}

/// The plan of `input` with the column `output`, the lump of each row by `call`, after the
/// columns of `input`.
pub fn plan(input: LogicalPlan, call: LumpCall, output: Column) -> Result<LogicalPlan> {
    let Decided {
        rows,
        decided,
        keys,
        d_keys,
        category_type,
        weight,
    } = decide(input.clone(), &call, Vec::new())?;
    let partition = &call.partition;
    let input_columns: Vec<Column> = input.schema().columns();

    // The weight of the rows of the lumped categories of each partition, and their number.
    let lumped_rows = LogicalPlanBuilder::from(rows.clone())
        .join_detailed(
            decided.clone(),
            JoinType::Inner,
            (
                (keys.iter().map(Column::from_name))
                    .chain([Column::from_name(CAT)])
                    .collect::<Vec<_>>(),
                (d_keys.iter().map(Column::from_name))
                    .chain([Column::from_name(D_CAT)])
                    .collect::<Vec<_>>(),
            ),
            Some(col(KEEP).is_false()),
            NullEquality::NullEqualsNull,
        )?
        .build()?;
    let other_keys: Vec<String> = (0..partition.len())
        .map(|i| format!("__lump_o_key_{i}"))
        .collect();
    let others = LogicalPlanBuilder::from(lumped_rows)
        .aggregate(
            (d_keys.iter().zip(&other_keys)).map(|(d, o)| col(d).alias(o)),
            vec![cast(weight, DataType::Float64).alias(OTHER)],
        )?
        .build()?;
    let count_keys: Vec<String> = (0..partition.len())
        .map(|i| format!("__lump_c_key_{i}"))
        .collect();
    let counts = LogicalPlanBuilder::from(decided.clone())
        .filter(col(KEEP).is_false())?
        .aggregate(
            (d_keys.iter().zip(&count_keys)).map(|(d, c)| col(d).alias(c)),
            vec![count_star().alias(LUMPED)],
        )?
        .build()?;
    // Without partition keys, `others` and `counts` are aggregates without groups, of one row.
    let decided = joined_by_keys(decided, others, &d_keys, &other_keys)?;
    let decided = joined_by_keys(decided, counts, &d_keys, &count_keys)?;

    // The rows with the lump of their category.
    let joined = LogicalPlanBuilder::from(rows)
        .join_detailed(
            decided,
            JoinType::Left,
            (
                (keys.iter().map(Column::from_name))
                    .chain([Column::from_name(CAT)])
                    .collect::<Vec<_>>(),
                (d_keys.iter().map(Column::from_name))
                    .chain([Column::from_name(D_CAT)])
                    .collect::<Vec<_>>(),
            ),
            None,
            NullEquality::NullEqualsNull,
        )?
        .build()?;
    // The lump, marked with its call and its input, so that `fuse` finds it.
    let lump = marked(
        lump_struct(CAT, &category_type, call.other.clone())?,
        call,
        input,
    )
    .alias_qualified(output.relation.clone(), output.name.clone());
    let mut exprs: Vec<Expr> = input_columns.into_iter().map(Expr::Column).collect();
    exprs.push(lump);
    LogicalPlanBuilder::from(joined).project(exprs)?.build()
}

/// `left` with the columns of the row of `right` whose `right_keys` equal its `left_keys`, or with
/// the one row of `right` without keys.
fn joined_by_keys(
    left: LogicalPlan,
    right: LogicalPlan,
    left_keys: &[String],
    right_keys: &[String],
) -> Result<LogicalPlan> {
    let builder = LogicalPlanBuilder::from(left);
    match left_keys.is_empty() {
        true => builder.cross_join(right)?.build(),
        false => builder
            .join_detailed(
                right,
                JoinType::Left,
                (
                    left_keys.iter().map(Column::from_name).collect::<Vec<_>>(),
                    right_keys.iter().map(Column::from_name).collect::<Vec<_>>(),
                ),
                None,
                NullEquality::NullEqualsNull,
            )?
            .build(),
    }
}

/// The field of the lump of a category of the type `category`.
pub fn lump_field(category: &DataType) -> Arc<Field> {
    Arc::new(Field::new(
        "lump",
        DataType::Struct(lump_fields(category)),
        true,
    ))
}

/// The placeholder window function of a lump in a projection, which [`plan_windows`] plans.
#[derive(Debug, PartialEq, Eq, Hash)]
struct LumpWindow {
    id: String,
    call: LumpCall,
    signature: Signature,
}

impl WindowUDFImpl for LumpWindow {
    fn name(&self) -> &str {
        &self.id
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn partition_evaluator(
        &self,
        _: PartitionEvaluatorArgs,
    ) -> Result<Box<dyn PartitionEvaluator>> {
        plan_err!("a lump runs only in a projection whose windows plan through lump::plan_windows")
    }
    fn field(&self, args: WindowUDFFieldArgs) -> Result<FieldRef> {
        let category = args.input_fields()[0].data_type();
        Ok(Arc::new(Field::new(
            args.name(),
            DataType::Struct(lump_fields(category)),
            true,
        )))
    }
}

/// The lump of `call` as a window expression over the rows, whose type is the lump's struct. The
/// window is a placeholder, which [`plan_windows`] plans. `id` names its function, so that the
/// windows of two calls over one category have different names.
pub fn window(call: LumpCall, id: &str) -> Expr {
    let category = call.category.clone();
    let partition = call.partition.clone();
    let udwf = WindowUDF::new_from_impl(LumpWindow {
        id: id.to_string(),
        call,
        signature: Signature::any(1, Volatility::Immutable),
    });
    let mut window = WindowFunction::new(
        WindowFunctionDefinition::WindowUDF(Arc::new(udwf)),
        vec![category],
    );
    window.params.partition_by = partition;
    window.params.window_frame = WindowFrame::new(None);
    Expr::from(window)
}

/// The call of the lump `expr`, a window of [`window`].
fn call_of(expr: &Expr) -> Option<&LumpCall> {
    let Expr::WindowFunction(function) = expr else {
        return None;
    };
    let WindowFunctionDefinition::WindowUDF(udwf) = &function.fun else {
        return None;
    };
    let inner: &dyn Any = udwf.inner().as_ref();
    inner.downcast_ref::<LumpWindow>().map(|lump| &lump.call)
}

/// `input` with the column of each lump of `windows`, and the other windows of `windows`, for a
/// window node over the result. The column of a lump is named after the window, as a window
/// node names its columns, so a projection over both reads each window as the column of its
/// schema name.
pub fn plan_windows(input: LogicalPlan, windows: Vec<Expr>) -> Result<(LogicalPlan, Vec<Expr>)> {
    let mut plan = input;
    let mut rest = Vec::new();
    for window in windows {
        match call_of(&window) {
            Some(call) => {
                let output = Column::from_name(window.schema_name().to_string());
                plan = self::plan(plan, call.clone(), output)?;
            }
            None => rest.push(window),
        }
    }
    Ok((plan, rest))
}

// ---------------------------------------------------------------------------------------------
// An aggregate grouped by a lump, in one read of the input.
// ---------------------------------------------------------------------------------------------

/// The lump `value` of the call `call` over `input`: `value` itself, through a function that
/// carries the call and the input, so that [`fuse`] finds the lump below an aggregate.
fn marked(value: Expr, call: LumpCall, input: LogicalPlan) -> Expr {
    ScalarUDF::new_from_impl(LumpMark {
        call,
        input,
        signature: Signature::any(1, Volatility::Immutable),
    })
    .call(vec![value])
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct LumpMark {
    call: LumpCall,
    input: LogicalPlan,
    signature: Signature,
}

impl ScalarUDFImpl for LumpMark {
    fn name(&self) -> &str {
        "lump"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(arg_types[0].clone())
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        Ok(args.arg_fields[0].clone())
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        Ok(args.args[0].clone())
    }
}

/// A column that stands for the lump in the expressions that [`fuse`] resolves.
const MARK: &str = "__lump_mark";

/// `exprs`, over the output of `plan`, as expressions over the input of the lump that `plan`
/// computes through projections, with the lump as the column [`MARK`], and the lump's call and
/// input; `None` where `plan` isn't such a lump.
fn resolve(
    plan: &LogicalPlan,
    exprs: Vec<Expr>,
) -> Result<Option<(Vec<Expr>, LumpCall, LogicalPlan)>> {
    use datafusion::common::tree_node::{Transformed, TreeNode};
    let (input, columns): (&LogicalPlan, Vec<Expr>) = match plan {
        LogicalPlan::Projection(projection) => {
            let expr: Vec<Expr> = (projection.expr.iter())
                .map(|e| e.clone().unalias_nested().data)
                .collect();
            (projection.input.as_ref(), expr)
        }
        LogicalPlan::SubqueryAlias(alias) => {
            let columns = (alias.input.schema().columns().into_iter())
                .map(Expr::Column)
                .collect();
            (alias.input.as_ref(), columns)
        }
        _ => return Ok(None),
    };
    let mark = columns.iter().find_map(|e| match e {
        Expr::ScalarFunction(function) => {
            let inner: &dyn Any = function.func.inner().as_ref();
            inner.downcast_ref::<LumpMark>()
        }
        _ => None,
    });
    let schema = plan.schema();
    let mut resolved = Vec::with_capacity(exprs.len());
    for expr in exprs {
        let mut missing = false;
        let expr = expr
            .transform_down(|e| match &e {
                Expr::Column(column) => match schema.index_of_column(column) {
                    Ok(index) => {
                        let found = &columns[index];
                        let is_mark = matches!(found, Expr::ScalarFunction(f)
                            if (f.func.inner().as_ref() as &dyn Any).is::<LumpMark>());
                        Ok(Transformed::new(
                            match is_mark {
                                true => col(MARK),
                                false => found.clone(),
                            },
                            true,
                            datafusion::common::tree_node::TreeNodeRecursion::Jump,
                        ))
                    }
                    Err(_) => {
                        missing = true;
                        Ok(Transformed::no(e))
                    }
                },
                _ => Ok(Transformed::no(e)),
            })?
            .data;
        if missing {
            return Ok(None);
        }
        resolved.push(expr);
    }
    match mark {
        // The lump's projection: its other columns are the input's.
        Some(mark) => Ok(Some((resolved, mark.call.clone(), mark.input.clone()))),
        None => resolve(input, resolved),
    }
}

/// The family of the state of `expr`, an aggregate that merges from the states of the
/// categories, and its argument.
fn mergeable(expr: &Expr) -> Option<(Family, Expr)> {
    let Expr::AggregateFunction(function) = expr else {
        return None;
    };
    let params = &function.params;
    if params.distinct
        || params.filter.is_some()
        || !params.order_by.is_empty()
        || params.null_treatment.is_some()
        || params.args.len() != 1
    {
        return None;
    }
    let family = match function.func.name() {
        "count" => Family::Count,
        "sum" => Family::Sum,
        "min" => Family::Min,
        "max" => Family::Max,
        "avg" => Family::Avg,
        "var" | "var_samp" => Family::VarSamp,
        "var_pop" => Family::VarPop,
        "stddev" | "stddev_samp" => Family::StddevSamp,
        "stddev_pop" => Family::StddevPop,
        _ => return None,
    };
    Some((family, params.args[0].clone()))
}

/// `plan`, an aggregate grouped by a lump, and by the lump's partition keys, computed from the
/// aggregate of the categories that the lump computes, so the plan reads its input once: each
/// category holds the states of the aggregates of its rows, the Other group merges the states
/// of its categories, and each value of the lump merges the states of its categories. `None`
/// where `plan` isn't such an aggregate, or one of its aggregates, or the lump's weight, doesn't
/// merge from states, as `COUNT`, `SUM`, `MIN`, `MAX`, `AVG` and the variances do.
pub fn fuse(plan: &LogicalPlan) -> Result<Option<LogicalPlan>> {
    let LogicalPlan::Aggregate(aggregate) = plan else {
        return Ok(None);
    };
    let groups = aggregate.group_expr.clone();
    let aggregates: Vec<Expr> = (aggregate.aggr_expr.iter())
        .map(|e| e.clone().unalias_nested().data)
        .collect();
    let exprs: Vec<Expr> = groups.iter().chain(&aggregates).cloned().collect();
    let Some((resolved, call, input)) = resolve(&aggregate.input, exprs)? else {
        return Ok(None);
    };
    let (groups, aggregates) = resolved.split_at(groups.len());
    // Each group is the lump or a key of its partition.
    let mut group_keys: Vec<Option<usize>> = Vec::new();
    for group in groups {
        match group {
            Expr::Column(column) if column.name == MARK => group_keys.push(None),
            group => match call.partition.iter().position(|p| p == group) {
                Some(index) => group_keys.push(Some(index)),
                None => return Ok(None),
            },
        }
    }
    if !group_keys.contains(&None) {
        return Ok(None);
    }
    let reads_mark = |e: &Expr| e.column_refs().iter().any(|c| c.name == MARK);
    let mut states = Vec::new();
    for (index, aggregate) in aggregates.iter().enumerate() {
        match mergeable(aggregate) {
            Some((family, arg)) if !reads_mark(&arg) => states.push((
                family,
                state(family, arg).alias(format!("__lump_state_{index}")),
            )),
            _ => return Ok(None),
        }
    }
    // The weight of the Other group merges the states of the weight's aggregates.
    let weight = call.weight.clone().unwrap_or_else(count_star);
    let weight_aggregates = find_aggregate_exprs([&weight]);
    let mut weight_states = Vec::new();
    for (index, aggregate) in weight_aggregates.iter().enumerate() {
        let Some((family, arg)) = mergeable(aggregate) else {
            return Ok(None);
        };
        weight_states.push((
            family,
            state(family, arg).alias(format!("__lump_wstate_{index}")),
        ));
    }
    let extra: Vec<Expr> = (states.iter().chain(&weight_states))
        .map(|(_, e)| e.clone())
        .collect();
    let Decided {
        decided,
        d_keys,
        category_type,
        ..
    } = decide(input, &call, extra)?;
    let other_weight = {
        use datafusion::common::tree_node::{Transformed, TreeNode};
        weight
            .transform_down(|e| match weight_aggregates.iter().position(|a| *a == e) {
                Some(index) => Ok(Transformed::yes(merge(
                    weight_states[index].0,
                    col(&format!("__lump_wstate_{index}")),
                ))),
                None => Ok(Transformed::no(e)),
            })?
            .data
    };
    // The weight and the number of the lumped categories of each partition, as windows over
    // the categories, so the categories are read once.
    let lumped_window = |aggregate: Expr| -> Result<Expr> {
        let Expr::AggregateFunction(function) = aggregate else {
            return plan_err!("an aggregate");
        };
        let mut window = WindowFunction::new(
            WindowFunctionDefinition::AggregateUDF(function.func.clone()),
            function.params.args.clone(),
        );
        window.params.partition_by = d_keys.iter().map(|k| col(k)).collect();
        window.params.filter = Some(Box::new(col(KEEP).is_false()));
        window.params.window_frame = WindowFrame::new(None);
        Ok(Expr::from(window))
    };
    let mut windows = Vec::new();
    let other_weight = {
        use datafusion::common::tree_node::{Transformed, TreeNode};
        other_weight
            .transform_up(|e| match e {
                Expr::AggregateFunction(_) => {
                    let window = lumped_window(e)?;
                    windows.push(window.clone());
                    Ok(Transformed::yes(Expr::Column(Column::from_name(
                        window.schema_name().to_string(),
                    ))))
                }
                e => Ok(Transformed::no(e)),
            })?
            .data
    };
    let count = lumped_window(count_star())?;
    windows.push(count.clone());
    let windowed = LogicalPlanBuilder::window_plan(decided.clone(), windows)?;
    let mut exprs: Vec<Expr> = decided
        .schema()
        .columns()
        .into_iter()
        .map(Expr::Column)
        .collect();
    exprs.push(cast(other_weight, DataType::Float64).alias(OTHER));
    exprs.push(Expr::Column(Column::from_name(count.schema_name().to_string())).alias(LUMPED));
    let categories = LogicalPlanBuilder::from(windowed).project(exprs)?.build()?;
    // Each value of the lump merges the states of its categories.
    let lump = lump_struct(D_CAT, &category_type, call.other.clone())?.alias(MARK);
    let final_groups: Vec<Expr> = (group_keys.iter().enumerate())
        .map(|(index, key)| match key {
            None => lump.clone(),
            Some(key) => col(&d_keys[*key]).alias(format!("__lump_group_{index}")),
        })
        .collect();
    let merges: Vec<Expr> = (states.iter().enumerate())
        .map(|(index, (family, _))| {
            merge(*family, col(&format!("__lump_state_{index}")))
                .alias(format!("__lump_merged_{index}"))
        })
        .collect();
    let merged = LogicalPlanBuilder::from(categories)
        .aggregate(final_groups, merges)?
        .build()?;
    // The columns of `plan`, with its names and types.
    let exprs: Vec<Expr> = (merged.schema().columns().into_iter())
        .zip(plan.schema().iter())
        .map(|(column, (qualifier, field))| {
            cast(Expr::Column(column), field.data_type().clone())
                .alias_qualified(qualifier.cloned(), field.name())
        })
        .collect();
    Ok(Some(
        LogicalPlanBuilder::from(merged).project(exprs)?.build()?,
    ))
}

/// An analyzer rule that plans each aggregate grouped by a lump through [`fuse`].
#[derive(Debug, Default)]
pub struct LumpAggregateFusion;

impl AnalyzerRule for LumpAggregateFusion {
    fn name(&self) -> &str {
        "lump_aggregate_fusion"
    }

    fn analyze(&self, plan: LogicalPlan, config: &ConfigOptions) -> Result<LogicalPlan> {
        use datafusion::common::tree_node::{Transformed, TreeNode};
        use datafusion::optimizer::analyzer::type_coercion::TypeCoercion;
        // The rule runs after DataFusion's coercion, so it coerces the plans that it builds. The
        // fusion only saves reads, so where it fails, the aggregate plans as it does without it.
        plan.transform_up(|plan| {
            let fused = fuse(&plan).and_then(|fused| {
                fused
                    .map(|fused| TypeCoercion::new().analyze(fused, config))
                    .transpose()
            });
            match fused {
                Ok(Some(fused)) => Ok(Transformed::yes(fused)),
                _ => Ok(Transformed::no(plan)),
            }
        })
        .map(|t| t.data)
    }
}
