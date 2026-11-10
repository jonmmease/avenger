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

/// A `|> CHART` operator: its mark items, its aggregate items, each with its name, the order of the
/// items, its group columns, and the group expressions, which the operators before it compute as
/// columns.
pub(crate) struct ChartOperator {
    pub(crate) marks: Vec<MarkItem>,
    pub(crate) aggregates: Vec<(Expr, Ident)>,
    pub(crate) items: Vec<Item>,
    pub(crate) groups: Vec<Expr>,
    pub(crate) extend: Option<PipeOperator>,
}

/// A mark item: the mark call and the name of its chart column.
pub(crate) struct MarkItem {
    pub(crate) mark: Expr,
    pub(crate) name: Ident,
}

/// An item of a `|> CHART`, in the order of the items: a mark item or an aggregate item, by its
/// index.
#[derive(Clone, Copy)]
pub(crate) enum Item {
    Mark(usize),
    Aggregate(usize),
}

/// The names of the relations of the join of a `|> CHART` with aggregate items or several mark
/// items: the aggregates, and the charts, the second `__avenger_charts_2`, and so on.
const AGGREGATES: &str = "__avenger_aggregates";
const CHARTS: &str = "__avenger_charts";

/// Whether `expr` is a call of a mark function.
fn is_mark(expr: &Expr) -> bool {
    matches!(expr, Expr::Function(function) if function_name(function).is_some_and(|name| avenger_chart::functions::mark(&name).is_some()))
}

/// The column of `expr`, named by `alias` or, without one, by its text.
fn named(expr: &Expr, alias: &Option<Ident>) -> Ident {
    match alias {
        Some(alias) => normalized(alias),
        None => Ident {
            value: expr.to_string(),
            quote_style: Some('"'),
            span: expr.span(),
        },
    }
}

/// `name` as the column that a reference to it finds: an unquoted name in lowercase, as DataFusion
/// normalizes identifiers, and a quoted name as written. A pipe `AGGREGATE` keeps the case of an
/// unquoted alias, so the operators that the pass writes name each column with its normalized
/// name.
fn normalized(name: &Ident) -> Ident {
    match name.quote_style {
        None => Ident::with_span(name.span, name.value.to_lowercase()),
        Some(_) => name.clone(),
    }
}

/// Reads the `|> CHART` operator `operator`: its items, of which the mark calls without `OVER` are
/// its charts, and the others aggregates, named by their aliases or their text, and the `GROUP BY`
/// list, whose columns group as they are and whose expressions are columns named by their aliases
/// or their text. One mark call is named `chart` without an alias, and each of several mark calls
/// needs one (D4). An item and a group take no `ASC` or `DESC`, and two take no one name.
pub(crate) fn read(operator: &PipeOperator) -> Result<ChartOperator> {
    let PipeOperator::Aggregate {
        full_table_exprs,
        group_by_expr,
    } = operator
    else {
        unreachable!("the marker is a pipe AGGREGATE")
    };
    let items = &full_table_exprs[1..];
    let mut marks: Vec<(&ExprWithAliasAndOrderBy, &Expr)> = Vec::new();
    let mut aggregates = Vec::new();
    let mut order = Vec::new();
    for item in items {
        unordered(item, "an item of CHART")?;
        let expr = &item.expr.expr;
        match is_mark(expr) {
            true => {
                order.push(Item::Mark(marks.len()));
                marks.push((item, expr));
            }
            false => {
                nested_mark(expr)?;
                order.push(Item::Aggregate(aggregates.len()));
                aggregates.push((expr.clone(), named(expr, &item.expr.alias)));
            }
        }
    }
    if marks.is_empty() {
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
    }
    let several = marks.len() > 1;
    let mut mark_items = Vec::new();
    for (item, mark) in marks {
        let Expr::Function(function) = mark else {
            unreachable!("a mark call is a call")
        };
        if function.over.is_some() {
            return Err(at_with_help(
                function.name.span(),
                format!("the mark call of CHART takes no OVER clause, so `{}` has none", function.name),
                "draw a chart for each group with GROUP BY, such as `|> CHART bar(x => species, y => count(*)) GROUP BY island`",
            ));
        }
        let name = match (&item.expr.alias, several) {
            (Some(alias), _) => normalized(alias),
            (None, false) => Ident::with_span(function.name.span(), CHART),
            (None, true) => {
                return Err(at_with_help(
                    function.name.span(),
                    format!(
                        "this CHART draws several charts, so `{}` needs a name",
                        function.name
                    ),
                    "name each chart, such as `|> CHART bar(...) AS bars, line(...) AS lines`",
                ))
            }
        };
        mark_items.push(MarkItem {
            mark: mark.clone(),
            name,
        });
    }
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
    let chart = ChartOperator {
        marks: mark_items,
        aggregates,
        items: order,
        groups,
        extend: (!extend.is_empty()).then_some(PipeOperator::Extend { exprs: extend }),
    };
    chart.unique_names()?;
    Ok(chart)
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
    /// The name of the item `item`.
    fn name(&self, item: Item) -> &Ident {
        match item {
            Item::Mark(index) => &self.marks[index].name,
            Item::Aggregate(index) => &self.aggregates[index].1,
        }
    }

    /// Each column of the output, an item or a group, has a name of its own, or the second of two
    /// columns of one name, in the order of the text, is an error at it. The output holds the
    /// columns without the qualifiers of the input, so `GROUP BY a.island, b.island` names two
    /// columns `island`.
    fn unique_names(&self) -> Result<()> {
        let items = (self.items.iter()).map(|item| {
            let name = self.name(*item);
            (name.value.clone(), name.span)
        });
        let groups = (self.groups.iter()).map(|group| (group_name(group).value, group.span()));
        let mut seen = std::collections::HashSet::new();
        for (name, span) in items.chain(groups) {
            if !seen.insert(name.clone()) {
                return Err(at_with_help(
                    span,
                    format!("`{name}` names two columns of this CHART"),
                    "give one of them an alias of its own, such as `GROUP BY a.island, b.island AS other_island`; a mark call without an alias names its column `chart`",
                ));
            }
        }
        Ok(())
    }

    /// Whether the operator joins relations: the aggregate items and the charts, or several charts.
    pub(crate) fn joins(&self) -> bool {
        !self.aggregates.is_empty() || self.marks.len() > 1
    }

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

    /// The condition of the join of the relation `aggregates`, the first, and the charts `charts`:
    /// each group is not distinct from its group, so a `NULL` group joins, or `TRUE` without groups.
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

    /// The output of the join, as `AGGREGATE` gives it: the groups, from the first relation, then
    /// the items in their order, the aggregates from `aggregates` and each chart from its relation
    /// of `charts`.
    pub(crate) fn projection(&self, aggregates: &Ident, charts: &[Ident]) -> Vec<SelectItem> {
        let column = |relation: &Ident, name: &Ident| {
            SelectItem::UnnamedExpr(Expr::CompoundIdentifier(vec![
                relation.clone(),
                name.clone(),
            ]))
        };
        let first = self.first(aggregates, charts);
        let groups = (self.groups.iter()).map(|group| column(first, &group_name(group)));
        let items = self.items.iter().map(|item| match item {
            Item::Mark(index) => column(&charts[*index], &self.marks[*index].name),
            Item::Aggregate(index) => column(aggregates, &self.aggregates[*index].1),
        });
        groups.chain(items).collect()
    }

    /// The first relation of the join: the aggregate items, or without them, the first chart.
    pub(crate) fn first<'a>(&self, aggregates: &'a Ident, charts: &'a [Ident]) -> &'a Ident {
        match self.aggregates.is_empty() {
            true => &charts[0],
            false => aggregates,
        }
    }

    /// The charts that join the first relation, with their names: every chart after the aggregate
    /// items, or every chart but the first.
    pub(crate) fn joined<'a>(&self, charts: &'a [Ident]) -> &'a [Ident] {
        match self.aggregates.is_empty() {
            true => &charts[1..],
            false => charts,
        }
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

/// The name of the column of the group `group`, the last part of its reference, normalized.
fn group_name(group: &Expr) -> Ident {
    match group {
        Expr::Identifier(ident) => normalized(ident),
        Expr::CompoundIdentifier(parts) => normalized(parts.last().expect("a part")),
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
    let span = chart.marks[0].name.span;
    let calls: Vec<TableFactor> = (chart.marks.iter())
        .map(|mark| chart_call(mark, &chart.groups, prefix.clone(), span))
        .collect();
    if !chart.joins() {
        let call = calls.into_iter().next().expect("a chart");
        *query.body = SetExpr::Select(Box::new(build::select(vec![build::wildcard()], Some(call))));
        return Ok(());
    }
    let aggregates = Ident::with_span(span, AGGREGATES);
    let charts: Vec<Ident> = (1..=calls.len())
        .map(|n| match n {
            1 => Ident::with_span(span, CHARTS),
            n => Ident::with_span(span, format!("{CHARTS}_{n}")),
        })
        .collect();
    let mut relations = (calls.into_iter().zip(&charts))
        .map(|(call, name)| aliased(call, name))
        .collect::<Vec<_>>()
        .into_iter();
    let first = match chart.aggregates.is_empty() {
        true => relations.next().expect("a chart"),
        false => {
            let mut input = prefix;
            input.pipe_operators.push(chart.aggregate());
            aliased(build::derived(input), &aggregates)
        }
    };
    let mut select = build::select(chart.projection(&aggregates, &charts), Some(first));
    let first_name = chart.first(&aggregates, &charts);
    for (relation, name) in relations.zip(chart.joined(&charts)) {
        select.from[0].joins.push(Join {
            relation,
            global: false,
            join_operator: JoinOperator::LeftOuter(JoinConstraint::On(
                chart.join_condition(first_name, name),
            )),
        });
    }
    *query.body = SetExpr::Select(Box::new(select));
    Ok(())
}

/// The `FROM`-position call of the mark planner of `mark` over `input`, grouped by `groups`.
fn chart_call(
    mark: &MarkItem,
    groups: &[Expr],
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
        FunctionArg::Unnamed(FunctionArgExpr::Expr(mark.mark.clone())),
        named(
            PARTITION_BY,
            Expr::Array(Array {
                elem: groups.to_vec(),
                named: false,
            }),
        ),
        named(NAME, Expr::Identifier(mark.name.clone())),
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
