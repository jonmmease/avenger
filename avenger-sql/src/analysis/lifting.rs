//! Statistics in the channels of a mark call: a mark call in an item of a pipe `AGGREGATE` whose
//! channels hold aggregates plans as a branch of the operator, the staged query that a user could
//! type for it.
//!
//! ```sql
//! FROM penguins
//! |> AGGREGATE bar(x => species, y => COUNT(*)) AS chart GROUP BY island
//! ```
//!
//! plans as
//!
//! ```sql
//! FROM penguins
//! |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, species
//! |> AGGREGATE bar(x => species, y => `COUNT(*)`) AS chart GROUP BY island
//! ```
//!
//! In a call that holds an aggregate, each channel that holds none is a key, and the call
//! aggregates once for each combination of the operator's groups and its keys. A channel that
//! holds an aggregate is a measure, which reads only keys, groups, and constants outside its
//! aggregates. Each channel's column takes the text of its value, which titles its axis or its
//! legend. The pass plans a branch for each such call over the operator's input, aggregated to
//! one row for each group, and joins the branches on the groups.

use avenger_chart::functions::Kind;
use avenger_sql_syntax::{ast, ast::AstNode, TextRange};
use datafusion::{
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Column, Result,
    },
    logical_expr::{
        expr::{AggregateFunction, ScalarFunction},
        lit,
        utils::find_aggregate_exprs,
        Expr, LogicalPlan, LogicalPlanBuilder, SortExpr,
    },
};

use super::{query::regroup, rows, Pass};

/// A mark call whose channels compute statistics, which an `AGGREGATE` plans as a branch.
#[derive(Clone, Debug)]
pub(crate) struct Lifting {
    /// The planned call, as the items of the `AGGREGATE` hold it.
    pub mark: Expr,
    /// The name of the column of each channel value, the value inside `scaled` or `direct`.
    pub names: Vec<(Expr, String)>,
}

/// A channel value of a mark call as the user wrote it: the planned value inside `scaled` or
/// `direct`, its range, and its text.
pub(crate) struct Written {
    pub value: Expr,
    pub range: TextRange,
    pub text: String,
}

/// The value of a channel argument, inside `scaled` or `direct`.
pub(crate) fn channel_value(arg: &Expr) -> &Expr {
    match arg {
        Expr::ScalarFunction(ScalarFunction { func, args })
            if !args.is_empty() && Kind::of(func.name()).is_some() =>
        {
            &args[0]
        }
        arg => arg,
    }
}

/// The text of the channel value `value`, inside `scaled` or `direct`, with its spaces collapsed.
pub(crate) fn channel_text(value: &ast::Expr) -> String {
    let inner = match value {
        ast::Expr::CallExpr(call) => {
            let kind = (call.path().and_then(|path| path.names()))
                .is_some_and(|names| {
                    matches!(names.as_slice(), [name] if Kind::of(&name.to_ascii_lowercase()).is_some())
                });
            let first =
                (call.arg_list().and_then(|list| list.arguments().next())).and_then(|argument| {
                    match argument {
                        ast::Argument::Arg(arg) => arg.expr(),
                        _ => None,
                    }
                });
            match (kind, first) {
                (true, Some(first)) => first,
                _ => value.clone(),
            }
        }
        _ => value.clone(),
    };
    let text = inner.syntax().text().to_string();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `expr` as an error names it: a column by its name, without its qualifier.
fn display(expr: &Expr) -> String {
    match expr {
        Expr::Column(column) => column.name.clone(),
        expr => expr.human_display().to_string(),
    }
}

/// Whether `expr` holds an aggregate function.
fn aggregates(expr: &Expr) -> bool {
    !find_aggregate_exprs([expr]).is_empty()
}

/// Whether `expr` reads a column.
fn reads_columns(expr: &Expr) -> bool {
    !expr.column_refs().is_empty()
}

/// `expr` with each aggregate and each of `known` replaced by a constant, so the columns that
/// remain are those that it reads outside them.
fn outside(expr: &Expr, known: &[&Expr]) -> Expr {
    expr.clone()
        .transform_down(|expr| {
            if matches!(expr, Expr::AggregateFunction(_)) || known.contains(&&expr) {
                return Ok(Transformed::new(lit(true), true, TreeNodeRecursion::Jump));
            }
            Ok(Transformed::no(expr))
        })
        .map(|transformed| transformed.data)
        .unwrap_or_else(|_| expr.clone())
}

impl Pass<'_> {
    /// Records `mark`, the planned call `name` at `range` of a mark in an item of a pipe
    /// `AGGREGATE` grouped by `groups`, when its channels or its `ORDER BY` hold an aggregate,
    /// after checking the grain of the call. `written` holds its channel values, and `order` the
    /// keys of its `ORDER BY` at `order_range`. Returns whether the call has no error.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lift(
        &mut self,
        name: &str,
        mark: &Expr,
        written: &[Written],
        order: &[SortExpr],
        order_range: Option<TextRange>,
        groups: Option<&[Expr]>,
        range: TextRange,
    ) -> bool {
        let lifted = written.iter().any(|value| aggregates(&value.value))
            || order.iter().any(|key| aggregates(&key.expr));
        if !lifted {
            return true;
        }
        let Some(groups) = groups else {
            self.error(
                range,
                format!(
                    "{name} computes statistics in its channels, which an AGGREGATE with \
                     GROUPING SETS, ROLLUP, or CUBE cannot hold"
                ),
            );
            return false;
        };
        let keys: Vec<&Expr> = (written.iter())
            .map(|value| &value.value)
            .filter(|value| !aggregates(value) && reads_columns(value))
            .collect();
        let known: Vec<&Expr> = keys.iter().copied().chain(groups).collect();
        let mut ok = true;
        for value in written.iter().filter(|value| aggregates(&value.value)) {
            let rest = outside(&value.value, &known);
            let mut columns: Vec<&Column> = rest.column_refs().into_iter().collect();
            columns.sort_by(|a, b| a.name.cmp(&b.name));
            if let Some(column) = columns.first() {
                self.error(
                    value.range,
                    format!(
                        "{} reads the column {} outside its aggregates, which is neither a key \
                         of {name} nor a group",
                        value.text, column.name
                    ),
                );
                ok = false;
            }
        }
        for key in order {
            if !aggregates(&key.expr) && !known.contains(&&key.expr) {
                self.error(
                    order_range.unwrap_or(range),
                    format!(
                        "The ORDER BY of {name} orders by {}, which is neither a channel of the \
                         call, a group, nor an aggregate",
                        display(&key.expr)
                    ),
                );
                ok = false;
            }
        }
        if !ok {
            return false;
        }
        let mut names: Vec<(Expr, String)> = Vec::new();
        for value in written {
            if names.iter().any(|(expr, _)| expr == &value.value) {
                continue;
            }
            let mut name = match &value.value {
                Expr::Column(column) => column.name.clone(),
                _ => value.text.clone(),
            };
            let taken = |name: &str, names: &[(Expr, String)]| names.iter().any(|(_, n)| n == name);
            if taken(&name, &names) {
                let base = name.clone();
                let mut number = 2;
                while taken(&name, &names) {
                    name = format!("{base}_{number}");
                    number += 1;
                }
            }
            names.push((value.value.clone(), name));
        }
        self.liftings.push(Lifting {
            mark: mark.clone(),
            names,
        });
        true
    }
}

/// The branch of `lifting` over `input`, grouped by `groups`, aliased `alias`: an aggregate by the
/// groups and the call's keys, a projection of each channel's value under its name, and the mark
/// aggregated by the groups. Its columns are `__group_0`, `__group_1`, and so on, then `__mark`.
pub(crate) fn branch(
    input: LogicalPlan,
    groups: &[Expr],
    lifting: &Lifting,
    alias: &str,
) -> Result<LogicalPlan> {
    let Expr::AggregateFunction(mark) = &lifting.mark else {
        unreachable!("a lifting holds a mark aggregate");
    };
    // The values of the channels and the keys of the call's `ORDER BY`, without constants.
    let values: Vec<&Expr> = (mark.params.args.iter())
        .map(channel_value)
        .filter(|value| reads_columns(value) || aggregates(value))
        .collect();
    let order: Vec<&Expr> = mark.params.order_by.iter().map(|key| &key.expr).collect();
    let mut group_exprs: Vec<Expr> = groups.to_vec();
    for value in &values {
        if !aggregates(value) && !group_exprs.contains(value) {
            group_exprs.push((*value).clone());
        }
    }
    let measured: Vec<&Expr> = values.iter().chain(&order).copied().collect();
    let aggregate_exprs = find_aggregate_exprs(measured.iter().copied());
    let aggregated = LogicalPlanBuilder::from(input)
        .aggregate(group_exprs.clone(), aggregate_exprs.clone())?
        .build()?;
    let output = aggregated.schema().columns();
    let grouped: Vec<(Expr, Column)> = group_exprs.iter().cloned().zip(output).collect();
    let staged = |expr: &Expr| -> Result<Expr> {
        regroup(rows::rebase(expr.clone(), &aggregate_exprs)?, &grouped)
    };
    // The groups, each channel value under its name, and the keys of the `ORDER BY`.
    let mut projection = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        projection.push(staged(group)?.alias(format!("__group_{index}")));
    }
    let mut named: Vec<(&Expr, String)> = Vec::new();
    for value in &values {
        if named.iter().any(|(expr, _)| expr == value) {
            continue;
        }
        let name = (lifting.names.iter())
            .find(|(expr, _)| expr == *value)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| value.human_display().to_string());
        projection.push(staged(value)?.alias(name.clone()));
        named.push((value, name));
    }
    for (index, key) in order.iter().enumerate() {
        projection.push(staged(key)?.alias(format!("__order_{index}")));
    }
    let projected = LogicalPlanBuilder::from(aggregated)
        .project(projection)?
        .build()?;
    // The call over the projection's columns.
    let column = |value: &Expr| -> Option<Expr> {
        (named.iter())
            .find(|(expr, _)| *expr == value)
            .map(|(_, name)| Expr::Column(Column::from_name(name.clone())))
    };
    let args = (mark.params.args.iter())
        .map(|arg| match arg {
            Expr::ScalarFunction(ScalarFunction { func, args })
                if !args.is_empty() && Kind::of(func.name()).is_some() =>
            {
                let mut args = args.clone();
                if let Some(column) = column(&args[0]) {
                    args[0] = column;
                }
                Expr::ScalarFunction(ScalarFunction {
                    func: func.clone(),
                    args,
                })
            }
            arg => column(arg).unwrap_or_else(|| arg.clone()),
        })
        .collect();
    let order_by = (mark.params.order_by.iter().enumerate())
        .map(|(index, key)| SortExpr {
            expr: Expr::Column(Column::from_name(format!("__order_{index}"))),
            ..key.clone()
        })
        .collect();
    let restaged = Expr::AggregateFunction(AggregateFunction::new_udf(
        mark.func.clone(),
        args,
        mark.params.distinct,
        mark.params.filter.clone(),
        order_by,
        mark.params.null_treatment,
    ));
    let group_columns: Vec<Expr> = (0..groups.len())
        .map(|index| Expr::Column(Column::from_name(format!("__group_{index}"))))
        .collect();
    LogicalPlanBuilder::from(projected)
        .aggregate(group_columns, vec![restaged.alias("__mark")])?
        .alias(alias)?
        .build()
}
