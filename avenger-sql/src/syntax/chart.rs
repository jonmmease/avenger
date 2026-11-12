//! `|> CHART`: the token pass writes `|> CHART <items> [GROUP BY ...]` as the pipe `AGGREGATE` of
//! the marker `__avenger_chart() AS __avenger_chart` and the items, so the parser reads the items,
//! their aliases, and the `GROUP BY` list of an `AGGREGATE`. The pass writes it as the
//! `FROM`-position call `__avenger_chart((<prefix>), <mark call>, partition_by => [...], name =>
//! <alias>)`, which the mark planner plans: one row for each group, with the group columns and the
//! chart. `CHART` is an operator of avenger-sql, so no upstream change removes the item.

use datafusion::{
    common::Result,
    sql::sqlparser::ast::{
        Array, Expr, ExprWithAliasAndOrderBy, FunctionArg, FunctionArgExpr, FunctionArgOperator,
        Ident, ObjectName, PipeOperator, Query, SelectItem, SetExpr, Spanned, TableFactor,
        TableFunctionArgs,
    },
};

use super::{arguments::function_name, build, split_prefix};
use crate::{
    chart::{CHART, MARKER, NAME, PARTITION_BY},
    error::{at, at_with_help},
};

/// Whether `operator` is the `AGGREGATE` of a `|> CHART`, whose first item is the marker.
pub(crate) fn is_marker(operator: &PipeOperator) -> bool {
    let PipeOperator::Aggregate {
        full_table_exprs, ..
    } = operator
    else {
        return false;
    };
    matches!(
        full_table_exprs.first().map(|item| &item.expr.expr),
        Some(Expr::Function(function)) if function_name(function).as_deref() == Some(MARKER)
    )
}

/// A `|> CHART` operator: its mark call, the name of its chart column, its group columns, and the
/// group expressions, which the operators before it compute as columns.
pub(crate) struct ChartOperator {
    pub(crate) mark: Expr,
    pub(crate) name: Ident,
    pub(crate) groups: Vec<Expr>,
    pub(crate) extend: Option<PipeOperator>,
}

/// Reads the `|> CHART` operator `operator`: one item, a mark call without `OVER`, named by its
/// alias or `chart`, and the `GROUP BY` list, whose columns group as they are and whose
/// expressions are columns named by their aliases or their text. An item and a group take no
/// `ASC` or `DESC`.
pub(crate) fn read(operator: &PipeOperator) -> Result<ChartOperator> {
    let PipeOperator::Aggregate {
        full_table_exprs,
        group_by_expr,
    } = operator
    else {
        unreachable!("the marker is a pipe AGGREGATE")
    };
    let items = &full_table_exprs[1..];
    let Some(item) = items.first() else {
        unreachable!("the parser reads an item after the marker")
    };
    if let Some(second) = items.get(1) {
        return Err(at(
            second.expr.expr.span(),
            "CHART takes one item, a mark call, such as `|> CHART bar(x => category, y => count(*))`",
        ));
    }
    unordered(item, "an item of CHART")?;
    let mark = item.expr.expr.clone();
    let Expr::Function(function) = &mark else {
        return Err(at(
            mark.span(),
            format!("CHART takes a mark call, not `{mark}`"),
        ));
    };
    if function.over.is_some() {
        return Err(at_with_help(
            function.name.span(),
            format!("the mark call of CHART takes no OVER clause, so `{}` has none", function.name),
            "draw a chart for each group with GROUP BY, such as `|> CHART bar(x => species, y => count(*)) GROUP BY island`",
        ));
    }
    let name =
        (item.expr.alias.clone()).unwrap_or_else(|| Ident::with_span(function.name.span(), CHART));
    let mut groups = Vec::new();
    let mut extend = Vec::new();
    for group in group_by_expr {
        unordered(group, "a group of CHART")?;
        let expr = &group.expr.expr;
        match (&group.expr.alias, expr) {
            (None, Expr::Identifier(_) | Expr::CompoundIdentifier(_)) => groups.push(expr.clone()),
            (alias, _) => {
                let alias = alias.clone().unwrap_or_else(|| Ident {
                    value: expr.to_string(),
                    quote_style: Some('"'),
                    span: expr.span(),
                });
                extend.push(SelectItem::ExprWithAlias {
                    expr: expr.clone(),
                    alias: alias.clone(),
                });
                groups.push(Expr::Identifier(alias));
            }
        }
    }
    Ok(ChartOperator {
        mark,
        name,
        groups,
        extend: (!extend.is_empty()).then_some(PipeOperator::Extend { exprs: extend }),
    })
}

/// `item` has no `ASC` or `DESC` and no `NULLS FIRST` or `NULLS LAST`, which pipe syntax allows
/// in an `AGGREGATE`.
fn unordered(item: &ExprWithAliasAndOrderBy, what: &str) -> Result<()> {
    if item.order_by.asc.is_some() || item.order_by.nulls_first.is_some() {
        return Err(at(
            item.expr.expr.span(),
            format!(
                "{what} takes no ASC, DESC, or NULLS order; order the rows after it with ORDER BY"
            ),
        ));
    }
    Ok(())
}

/// Writes the `|> CHART` at `index` of `query` as the `FROM`-position call of the mark planner over
/// the prefix, the query before it, with the columns of its group expressions.
pub(super) fn split(query: &mut Query, index: usize) -> Result<()> {
    let chart = read(&query.pipe_operators[index])?;
    let (mut prefix, _) = split_prefix(query, index);
    prefix.pipe_operators.extend(chart.extend);
    let span = chart.name.span;
    let named = |name: &str, value: Expr| FunctionArg::Named {
        name: Ident::with_span(span, name),
        arg: FunctionArgExpr::Expr(value),
        operator: FunctionArgOperator::RightArrow,
    };
    let args = vec![
        FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Subquery(Box::new(prefix)))),
        FunctionArg::Unnamed(FunctionArgExpr::Expr(chart.mark)),
        named(
            PARTITION_BY,
            Expr::Array(Array {
                elem: chart.groups,
                named: false,
            }),
        ),
        named(NAME, Expr::Identifier(chart.name)),
    ];
    let table = TableFactor::Table {
        name: ObjectName::from(vec![Ident::with_span(span, MARKER)]),
        alias: None,
        args: Some(TableFunctionArgs {
            args,
            settings: None,
        }),
        with_hints: Vec::new(),
        version: None,
        with_ordinality: false,
        partitions: Vec::new(),
        json_path: None,
        sample: None,
        index_hints: Vec::new(),
    };
    *query.body = SetExpr::Select(Box::new(build::select(
        vec![build::wildcard()],
        Some(table),
    )));
    Ok(())
}

/// The mark call of the chart call `table`, which the pass does not lower: the mark planner reads
/// its channels as they are written.
pub(super) fn mark_call(table: &mut TableFactor) -> Option<&mut FunctionArg> {
    match table {
        TableFactor::Table {
            name,
            args: Some(args),
            ..
        } if name.to_string() == MARKER => args.args.get_mut(1),
        _ => None,
    }
}
