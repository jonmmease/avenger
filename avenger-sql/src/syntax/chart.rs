//! `|> CHART`: the token pass writes `|> CHART <items> [GROUP BY ...]` as the pipe `AGGREGATE` of
//! the marker `__avenger_chart() AS __avenger_chart` and the items, so the parser reads the items,
//! their aliases, and the `GROUP BY` list of an `AGGREGATE`. The pass writes it as the
//! `FROM`-position call `__avenger_chart((<prefix>), <mark call>, partition_by => [...], name =>
//! <alias>)`, which the mark planner plans: one row for each group, with the group columns and the
//! chart. `CHART` is an operator of avenger-sql, so no upstream change removes the item.
//!
//! The other items are aggregates, a pipe `AGGREGATE` of their own over a copy of the prefix,
//! which the charts join on the group columns: `LEFT JOIN ... ON <group> IS NOT DISTINCT FROM
//! <group>`, so a `NULL` group has its chart, and the output has the columns of an `AGGREGATE`.

use std::ops::ControlFlow;

use datafusion::{
    common::Result,
    sql::sqlparser::ast::{
        Array, BinaryOperator, Expr, ExprWithAlias, ExprWithAliasAndOrderBy, FunctionArg,
        FunctionArgExpr, FunctionArgOperator, Ident, Join, JoinConstraint, JoinOperator,
        ObjectName, OrderByOptions, PipeOperator, Query, SelectItem, SetExpr, Spanned, TableAlias,
        TableFactor, TableFunctionArgs, Value, Visit, Visitor,
    },
};

use super::{arguments::function_name, build, split_prefix};
use crate::{
    chart::{CHART, MARKER, NAME, PARTITION_BY},
    error::{at, at_with_help},
    registry::Registry,
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
/// group expressions, which the operators before it compute as columns, and its aggregate items,
/// each with its name, with the position of the mark call among the items.
pub(crate) struct ChartOperator {
    pub(crate) mark: Expr,
    pub(crate) name: Ident,
    pub(crate) groups: Vec<Expr>,
    pub(crate) extend: Option<PipeOperator>,
    pub(crate) aggregates: Vec<(Expr, Ident)>,
    pub(crate) mark_position: usize,
}

/// The name of a relation of the join of a `|> CHART` with aggregate items: the aggregates, and the
/// charts.
const AGGREGATES: &str = "__avenger_aggregates";
const CHARTS: &str = "__avenger_charts";

/// Whether `expr` is a call of a mark function.
fn is_mark(expr: &Expr) -> bool {
    matches!(expr, Expr::Function(function) if function_name(function).is_some_and(|name| avenger_chart::functions::mark(&name).is_some()))
}

/// The column of `expr`, named by `alias` or, without one, by its text.
fn named(expr: &Expr, alias: &Option<Ident>) -> Ident {
    alias.clone().unwrap_or_else(|| Ident {
        value: expr.to_string(),
        quote_style: Some('"'),
        span: expr.span(),
    })
}

/// Reads the `|> CHART` operator `operator`: its items, of which one is a mark call without
/// `OVER`, named by its alias or `chart`, and the others aggregates, named by their aliases or
/// their text, and the `GROUP BY` list, whose columns group as they are and whose expressions are
/// columns named by their aliases or their text. An item and a group take no `ASC` or `DESC`.
pub(crate) fn read(operator: &PipeOperator) -> Result<ChartOperator> {
    let PipeOperator::Aggregate {
        full_table_exprs,
        group_by_expr,
    } = operator
    else {
        unreachable!("the marker is a pipe AGGREGATE")
    };
    let items = &full_table_exprs[1..];
    let mut mark = None;
    let mut aggregates = Vec::new();
    for item in items {
        unordered(item, "an item of CHART")?;
        let expr = &item.expr.expr;
        if !is_mark(expr) {
            nested_mark(expr)?;
            aggregates.push((expr.clone(), named(expr, &item.expr.alias)));
            continue;
        }
        if mark.is_some() {
            return Err(at(
                expr.span(),
                "CHART takes one mark call; draw another chart in a CHART of its own",
            ));
        }
        mark = Some((item, aggregates.len()));
    }
    let Some((item, mark_position)) = mark else {
        let first = &items[0].expr.expr;
        return Err(match first {
            Expr::Function(function) => at(
                function.name.span(),
                format!(
                    "`{}` is not a mark function, and CHART takes a mark call",
                    function.name
                ),
            ),
            _ => at(
                first.span(),
                format!("CHART takes a mark call, not `{first}`"),
            ),
        });
    };
    let mark = item.expr.expr.clone();
    let Expr::Function(function) = &mark else {
        unreachable!("a mark call is a call")
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
                let alias = named(expr, alias);
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
        aggregates,
        mark_position,
    })
}

/// An aggregate item holds no call of a mark function, which is the complete value of its item.
fn nested_mark(expr: &Expr) -> Result<()> {
    struct Find;
    impl Visitor for Find {
        type Break = Expr;
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Expr> {
            match is_mark(expr) {
                true => ControlFlow::Break(expr.clone()),
                false => ControlFlow::Continue(()),
            }
        }
    }
    match Visit::visit(expr, &mut Find) {
        ControlFlow::Break(Expr::Function(function)) => Err(at(
            function.name.span(),
            format!(
                "a mark call is the complete value of an item of CHART, so `{}` takes no expression around it",
                function.name
            ),
        )),
        _ => Ok(()),
    }
}

impl ChartOperator {
    /// Each aggregate item reads columns only inside its aggregates, or groups, and calls no window
    /// function, as an item of `AGGREGATE` does. DataFusion 55.2 checks neither in a pipe
    /// `AGGREGATE`, which fails at execution, so the pass checks them at their text.
    pub(crate) fn check_aggregates(&self, registry: &dyn Registry) -> Result<()> {
        let groups: Vec<String> = (self.groups.iter())
            .map(|group| group_name(group).value.to_lowercase())
            .collect();
        for (expr, _) in &self.aggregates {
            let mut check = CheckItem {
                registry,
                groups: &groups,
                aggregates: 0,
                queries: 0,
                inside: Vec::new(),
            };
            if let ControlFlow::Break(error) = Visit::visit(expr, &mut check) {
                return Err(error);
            }
        }
        Ok(())
    }

    /// The pipe `AGGREGATE` of the aggregate items over the groups.
    pub(crate) fn aggregate(&self) -> PipeOperator {
        let plain = |expr: Expr, alias: Option<Ident>| ExprWithAliasAndOrderBy {
            expr: ExprWithAlias { expr, alias },
            order_by: OrderByOptions {
                asc: None,
                nulls_first: None,
            },
        };
        PipeOperator::Aggregate {
            full_table_exprs: (self.aggregates.iter())
                .map(|(expr, name)| plain(expr.clone(), Some(name.clone())))
                .collect(),
            group_by_expr: (self.groups.iter())
                .map(|group| plain(group.clone(), None))
                .collect(),
        }
    }

    /// The condition of the join of the aggregates `aggregates` and the charts `charts`: each group
    /// is not distinct from its group, so a `NULL` group joins, or `TRUE` without groups.
    pub(crate) fn join_condition(&self, aggregates: &Ident, charts: &Ident) -> Expr {
        (self.groups.iter())
            .map(|group| {
                let name = group_name(group);
                Expr::IsNotDistinctFrom(
                    Box::new(Expr::CompoundIdentifier(vec![
                        aggregates.clone(),
                        name.clone(),
                    ])),
                    Box::new(Expr::CompoundIdentifier(vec![charts.clone(), name])),
                )
            })
            .reduce(|left, right| Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::And,
                right: Box::new(right),
            })
            .unwrap_or_else(|| Expr::value(Value::Boolean(true)))
    }

    /// The output of the join, as `AGGREGATE` gives it: the groups, then the items in their order,
    /// the aggregates from `aggregates` and the chart from `charts`.
    pub(crate) fn projection(&self, aggregates: &Ident, charts: &Ident) -> Vec<SelectItem> {
        let column = |relation: &Ident, name: Ident| {
            SelectItem::UnnamedExpr(Expr::CompoundIdentifier(vec![relation.clone(), name]))
        };
        let mut items: Vec<SelectItem> = (self.groups.iter())
            .map(|group| column(aggregates, group_name(group)))
            .collect();
        let mut names: Vec<SelectItem> = (self.aggregates.iter())
            .map(|(_, name)| column(aggregates, name.clone()))
            .collect();
        names.insert(self.mark_position, column(charts, self.name.clone()));
        items.extend(names);
        items
    }
}

/// The check of an aggregate item: the depth of the aggregates and of the subqueries that the visit
/// is in, and whether each expression that it is in is an aggregate.
struct CheckItem<'a> {
    registry: &'a dyn Registry,
    groups: &'a [String],
    aggregates: usize,
    queries: usize,
    inside: Vec<bool>,
}

impl Visitor for CheckItem<'_> {
    type Break = datafusion::common::DataFusionError;

    fn pre_visit_query(&mut self, _query: &Query) -> ControlFlow<Self::Break> {
        self.queries += 1;
        ControlFlow::Continue(())
    }

    fn post_visit_query(&mut self, _query: &Query) -> ControlFlow<Self::Break> {
        self.queries -= 1;
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        let mut aggregate = false;
        if self.queries == 0 {
            match expr {
                Expr::Function(function) if function.over.is_some() => {
                    return ControlFlow::Break(at_with_help(
                        function.name.span(),
                        format!(
                            "an item of CHART is an aggregate, not the window function `{}`",
                            function.name
                        ),
                        "compute the window after CHART, with EXTEND",
                    ));
                }
                Expr::Function(function) => {
                    aggregate = function_name(function)
                        .is_some_and(|name| self.registry.is_aggregate(&name));
                }
                Expr::Identifier(_) | Expr::CompoundIdentifier(_) if self.aggregates == 0 => {
                    let name = match expr {
                        Expr::Identifier(ident) => ident.value.to_lowercase(),
                        _ => group_name(expr).value.to_lowercase(),
                    };
                    if !self.groups.contains(&name) {
                        return ControlFlow::Break(at_with_help(
                            expr.span(),
                            format!(
                                "`{expr}` is neither a group of this CHART nor inside an aggregate"
                            ),
                            format!("group by it, or aggregate it, for example max({expr})"),
                        ));
                    }
                }
                _ => {}
            }
        }
        self.aggregates += usize::from(aggregate);
        self.inside.push(aggregate);
        ControlFlow::Continue(())
    }

    fn post_visit_expr(&mut self, _expr: &Expr) -> ControlFlow<Self::Break> {
        if self.inside.pop() == Some(true) {
            self.aggregates -= 1;
        }
        ControlFlow::Continue(())
    }
}

/// The name of the column of the group `group`, the last part of its reference.
fn group_name(group: &Expr) -> Ident {
    match group {
        Expr::Identifier(ident) => ident.clone(),
        Expr::CompoundIdentifier(parts) => parts.last().expect("a part").clone(),
        _ => unreachable!("a group is a column"),
    }
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
/// the prefix, the query before it, with the columns of its group expressions. With aggregate
/// items, the call joins the pipe `AGGREGATE` of the items over a copy of the prefix.
pub(super) fn split(query: &mut Query, index: usize, registry: &dyn Registry) -> Result<()> {
    let chart = read(&query.pipe_operators[index])?;
    chart.check_aggregates(registry)?;
    let (mut prefix, _) = split_prefix(query, index);
    prefix.pipe_operators.extend(chart.extend.clone());
    let span = chart.name.span;
    let charts = chart_call(&chart, prefix.clone(), span);
    let select = match chart.aggregates.is_empty() {
        true => build::select(vec![build::wildcard()], Some(charts)),
        false => {
            let (aggregates, charts_name) = (
                Ident::with_span(span, AGGREGATES),
                Ident::with_span(span, CHARTS),
            );
            let mut input = prefix;
            input.pipe_operators.push(chart.aggregate());
            let mut select = build::select(
                chart.projection(&aggregates, &charts_name),
                Some(aliased(build::derived(input), &aggregates)),
            );
            select.from[0].joins.push(Join {
                relation: aliased(charts, &charts_name),
                global: false,
                join_operator: JoinOperator::LeftOuter(JoinConstraint::On(
                    chart.join_condition(&aggregates, &charts_name),
                )),
            });
            select
        }
    };
    *query.body = SetExpr::Select(Box::new(select));
    Ok(())
}

/// The `FROM`-position call of the mark planner of `chart` over `input`.
fn chart_call(
    chart: &ChartOperator,
    input: Query,
    span: datafusion::sql::sqlparser::tokenizer::Span,
) -> TableFactor {
    let named = |name: &str, value: Expr| FunctionArg::Named {
        name: Ident::with_span(span, name),
        arg: FunctionArgExpr::Expr(value),
        operator: FunctionArgOperator::RightArrow,
    };
    let args = vec![
        FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Subquery(Box::new(input)))),
        FunctionArg::Unnamed(FunctionArgExpr::Expr(chart.mark.clone())),
        named(
            PARTITION_BY,
            Expr::Array(Array {
                elem: chart.groups.clone(),
                named: false,
            }),
        ),
        named(NAME, Expr::Identifier(chart.name.clone())),
    ];
    TableFactor::Table {
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
    }
}

/// `table` with the alias `alias`.
pub(crate) fn aliased(mut table: TableFactor, alias: &Ident) -> TableFactor {
    let name = Some(TableAlias {
        explicit: true,
        name: alias.clone(),
        columns: Vec::new(),
        at: None,
    });
    match &mut table {
        TableFactor::Table { alias, .. } | TableFactor::Derived { alias, .. } => *alias = name,
        _ => unreachable!("a table or a derived table"),
    }
    table
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
