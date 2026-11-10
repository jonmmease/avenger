//! The order of the children of `layer`, `hconcat`, and `vconcat`: an analyzer rule that gives
//! each call an order of its rows that DataFusion keeps, as the spec orders the children of a
//! composition (`r[panel.order.ordinal]`).
//!
//! The rows of an aggregate arrive in no defined order, so the rule gives each call these keys
//! after the keys of its own `ORDER BY`:
//!
//! 1. The branch ordinal: the position of the branch of a `UNION ALL` that a row comes from, when
//!    a `UNION ALL` feeds the call through projections, filters, sorts, limits, aliases, and
//!    grouped aggregates. A grouped aggregate gives each group the smallest ordinal of its rows
//!    (`r[panel.order.survive]`). A `UNION` without `ALL` is a distinct union, so its rows have no
//!    ordinal (`r[panel.order.union-all]`).
//! 2. Each other column of the call's input that holds no charts and that the aggregate does not
//!    group by, ascending with `NULL` last, in the order of the input schema.
//!
//! The facets order their charts by their keys, so the rule leaves them as they are.

use std::sync::Arc;

use avenger_chart::EXTENSION_NAME;
use datafusion::{
    common::{config::ConfigOptions, tree_node::Transformed, Column, Result},
    functions_aggregate::expr_fn::min,
    logical_expr::{
        expr::AggregateFunction, lit, Aggregate, Expr, LogicalPlan, Projection, SortExpr, Union,
    },
    optimizer::AnalyzerRule,
};

/// The name of the column of the branch ordinal.
const BRANCH: &str = "__avenger_branch";

/// The compositions whose children the rule orders.
const ORDERED: [&str; 3] = ["layer", "hconcat", "vconcat"];

/// The analyzer rule of the order of the children of compositions.
#[derive(Debug, Default)]
pub(crate) struct CompositionOrder;

impl AnalyzerRule for CompositionOrder {
    fn analyze(&self, plan: LogicalPlan, _config: &ConfigOptions) -> Result<LogicalPlan> {
        plan.transform_up_with_subqueries(|plan| match plan {
            LogicalPlan::Aggregate(aggregate) if composes(&aggregate) => order(aggregate),
            plan => Ok(Transformed::no(plan)),
        })
        .map(|transformed| transformed.data)
    }

    fn name(&self) -> &str {
        "avenger_composition_order"
    }
}

/// Whether `aggregate` calls a composition whose children the rule orders.
fn composes(aggregate: &Aggregate) -> bool {
    (aggregate.aggr_expr.iter()).any(|expr| composition(expr).is_some())
}

/// The call of a composition that `expr` is, under its alias.
fn composition(expr: &Expr) -> Option<&AggregateFunction> {
    match expr {
        Expr::Alias(alias) => composition(&alias.expr),
        Expr::AggregateFunction(call) if ORDERED.contains(&call.func.name()) => Some(call),
        _ => None,
    }
}

/// The aggregate `aggregate`, whose composition calls take the keys of the rule after their own.
fn order(aggregate: Aggregate) -> Result<Transformed<LogicalPlan>> {
    let Aggregate {
        input,
        group_expr,
        aggr_expr,
        ..
    } = aggregate;
    let (input, branch) = match with_branch(&input)? {
        Some(input) => (Arc::new(input), true),
        None => (input, false),
    };
    let grouped: Vec<Column> = group_expr
        .iter()
        .flat_map(|expr| expr.column_refs())
        .cloned()
        .collect();
    let mut keys: Vec<SortExpr> = Vec::new();
    if branch {
        keys.push(Expr::Column(Column::new_unqualified(BRANCH)).sort(true, false));
    }
    for (qualifier, field) in input.schema().iter() {
        let column = Column::new(qualifier.cloned(), field.name());
        let chart = field
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str)
            == Some(EXTENSION_NAME);
        if field.name() == BRANCH || chart || grouped.contains(&column) {
            continue;
        }
        keys.push(Expr::Column(column).sort(true, false));
    }
    let aggr_expr = (aggr_expr.into_iter())
        .map(|expr| add_keys(expr, &keys))
        .collect::<Result<Vec<_>>>()?;
    let aggregate = Aggregate::try_new(input, group_expr, aggr_expr)?;
    Ok(Transformed::yes(LogicalPlan::Aggregate(aggregate)))
}

/// `expr` with the keys `keys` after the keys of its composition call's `ORDER BY`.
fn add_keys(expr: Expr, keys: &[SortExpr]) -> Result<Expr> {
    match expr {
        Expr::Alias(mut alias) => {
            *alias.expr = add_keys(*alias.expr, keys)?;
            Ok(Expr::Alias(alias))
        }
        Expr::AggregateFunction(mut call) if ORDERED.contains(&call.func.name()) => {
            call.params.order_by.extend(keys.iter().cloned());
            Ok(Expr::AggregateFunction(call))
        }
        expr => Ok(expr),
    }
}

/// `plan` with the column of the branch ordinal, when a `UNION ALL` feeds it through the nodes
/// that keep the ordinal of a row.
fn with_branch(plan: &LogicalPlan) -> Result<Option<LogicalPlan>> {
    let branch = || Expr::Column(Column::new_unqualified(BRANCH));
    Ok(Some(match plan {
        LogicalPlan::Union(_) => number(plan, &mut 0)?,
        LogicalPlan::Projection(projection) => {
            let Some(input) = with_branch(&projection.input)? else {
                return Ok(None);
            };
            let mut exprs = projection.expr.clone();
            exprs.push(branch());
            LogicalPlan::Projection(Projection::try_new(exprs, Arc::new(input))?)
        }
        LogicalPlan::Aggregate(aggregate) if !aggregate.group_expr.is_empty() => {
            let Some(input) = with_branch(&aggregate.input)? else {
                return Ok(None);
            };
            let mut aggr_expr = aggregate.aggr_expr.clone();
            aggr_expr.push(min(branch()).alias(BRANCH));
            LogicalPlan::Aggregate(Aggregate::try_new(
                Arc::new(input),
                aggregate.group_expr.clone(),
                aggr_expr,
            )?)
        }
        LogicalPlan::Filter(_)
        | LogicalPlan::Sort(_)
        | LogicalPlan::Limit(_)
        | LogicalPlan::SubqueryAlias(_) => {
            let [input] = plan.inputs()[..] else {
                return Ok(None);
            };
            let Some(input) = with_branch(input)? else {
                return Ok(None);
            };
            plan.with_new_exprs(plan.expressions(), vec![input])?
        }
        _ => return Ok(None),
    }))
}

/// `plan` with the column of the branch ordinal on each of its branches, numbered from `next` in
/// order. A union inside a union, also under the projections and the aliases that `UNION ALL BY
/// NAME` writes over a union, numbers its branches among all of them, so `a UNION ALL BY NAME b
/// UNION ALL BY NAME c` gives `a`, `b`, and `c` the ordinals 0, 1, and 2.
fn number(plan: &LogicalPlan, next: &mut i64) -> Result<LogicalPlan> {
    let branch = || Expr::Column(Column::new_unqualified(BRANCH));
    Ok(match plan {
        LogicalPlan::Union(union) => {
            let inputs = (union.inputs.iter())
                .map(|input| number(input, next).map(Arc::new))
                .collect::<Result<Vec<_>>>()?;
            LogicalPlan::Union(Union::try_new_with_loose_types(inputs)?)
        }
        LogicalPlan::Projection(projection) if holds_union(plan) => {
            let input = number(&projection.input, next)?;
            let mut exprs = projection.expr.clone();
            exprs.push(branch());
            LogicalPlan::Projection(Projection::try_new(exprs, Arc::new(input))?)
        }
        LogicalPlan::SubqueryAlias(alias) if holds_union(plan) => {
            let input = number(&alias.input, next)?;
            plan.with_new_exprs(plan.expressions(), vec![input])?
        }
        branch_plan => {
            let mut exprs: Vec<Expr> = (branch_plan.schema().iter())
                .map(|(qualifier, field)| {
                    Expr::Column(Column::new(qualifier.cloned(), field.name()))
                })
                .collect();
            exprs.push(lit(*next).alias(BRANCH));
            *next += 1;
            LogicalPlan::Projection(Projection::try_new(exprs, Arc::new(branch_plan.clone()))?)
        }
    })
}

/// Whether `plan` is a union, or projections and aliases over one.
fn holds_union(plan: &LogicalPlan) -> bool {
    match plan {
        LogicalPlan::Union(_) => true,
        LogicalPlan::Projection(projection) => holds_union(&projection.input),
        LogicalPlan::SubqueryAlias(alias) => holds_union(&alias.input),
        _ => false,
    }
}
