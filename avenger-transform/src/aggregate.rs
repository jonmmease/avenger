use datafusion::{
    common::{
        plan_err,
        tree_node::{Transformed, TreeNode},
        Column, Result,
    },
    logical_expr::{
        expr_rewriter::normalize_col, utils::find_aggregate_exprs, Expr, LogicalPlan,
        LogicalPlanBuilder,
    },
};

/// Group rows and compute aliased aggregate expressions, including finishing arithmetic.
/// Empty input produces zero rows even without grouping keys, matching Vega's drop behavior.
/// Native aggregate functions retain their own value semantics.
pub fn aggregate(
    input: LogicalPlan,
    group_by: Vec<Expr>,
    measures: Vec<Expr>,
) -> Result<LogicalPlan> {
    if measures.is_empty() {
        return plan_err!("Aggregate requires at least one aliased measure");
    }
    for expr in &group_by {
        if !matches!(expr, Expr::Column(_) | Expr::Alias(_)) {
            return plan_err!("Computed aggregate grouping expressions require an alias");
        }
    }
    if measures.iter().any(|e| !matches!(e, Expr::Alias(_))) {
        return plan_err!("Aggregate measures require explicit aliases");
    }
    let group_by = group_by
        .into_iter()
        .map(|e| normalize_col(e, &input))
        .collect::<Result<Vec<_>>>()?;
    let measures = measures
        .into_iter()
        .map(|e| normalize_col(e, &input))
        .collect::<Result<Vec<_>>>()?;
    let mut names = std::collections::HashSet::new();
    for e in group_by.iter().chain(&measures) {
        let name = output_name(e);
        if !names.insert(name.clone()) {
            return plan_err!("Aggregate output field {name} is repeated");
        }
    }
    let mut prefix = "__avenger_".to_string();
    while input
        .schema()
        .fields()
        .iter()
        .any(|f| f.name().starts_with(&prefix))
        || names.iter().any(|name| name.starts_with(&prefix))
    {
        prefix.push('_');
    }
    let mut calls = find_aggregate_exprs(&measures);
    let rows = crate::expr_fn::count();
    let row_count = if group_by.is_empty() {
        Some(calls.iter().position(|e| e == &rows).unwrap_or_else(|| {
            calls.push(rows);
            calls.len() - 1
        }))
    } else {
        None
    };
    let mut bindings = Vec::new();
    let groups = group_by
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let value = match e {
                Expr::Alias(a) => *a.expr.clone(),
                e => e.clone(),
            };
            let name = format!("{prefix}group_{i}");
            bindings.push((value.clone(), Expr::Column(Column::from_name(&name))));
            value.alias(name)
        })
        .collect::<Vec<_>>();
    let aggregates = calls
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let name = format!("{prefix}measure_{i}");
            bindings.push((e.clone(), Expr::Column(Column::from_name(&name))));
            e.clone().alias(name)
        })
        .collect::<Vec<_>>();
    let mut plan = LogicalPlanBuilder::from(input).aggregate(groups, aggregates)?;
    if let Some(row_count) = row_count {
        plan = plan.filter(
            Expr::Column(Column::from_name(format!("{prefix}measure_{row_count}")))
                .gt(datafusion::logical_expr::lit(0_i64)),
        )?;
    }
    let output = group_by
        .into_iter()
        .chain(measures)
        .map(|e| {
            let name = output_name(&e);
            let value = match e {
                Expr::Alias(a) => *a.expr,
                e => e,
            };
            let value = value
                .transform_down(|e| {
                    Ok(match bindings.iter().find(|(original, _)| original == &e) {
                        Some((_, column)) => Transformed::new(
                            column.clone(),
                            true,
                            datafusion::common::tree_node::TreeNodeRecursion::Jump,
                        ),
                        None => Transformed::no(e),
                    })
                })?
                .data;
            Ok(value.alias(name))
        })
        .collect::<Result<Vec<_>>>()?;
    plan.project(output)?.build()
}

fn output_name(expr: &Expr) -> String {
    match expr {
        Expr::Column(column) => column.name.clone(),
        Expr::Alias(alias) => alias.name.clone(),
        _ => unreachable!("output names were validated"),
    }
}
