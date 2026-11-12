//! The places and the names of mark calls. A mark call builds a chart of the rows of a group, so
//! it is valid only in an item of a pipe `AGGREGATE`, outside the arguments of another aggregate,
//! and without `OVER`. A mark call anywhere else, flat or not, is an error at its text, whose help
//! shows the `AGGREGATE` form.
//!
//! An item of such an `AGGREGATE` without an alias that holds a mark call and is a call takes the
//! name of its function, in lowercase, so `bar(...)` is `bar` and `layer(bar(...), rule(...))` is
//! `layer`, and another item that holds a mark call, such as a `CASE`, needs an alias. The other
//! items keep the names that DataFusion gives them, and each column of the output, an item or a
//! group, has a name of its own.

use std::{collections::HashSet, ops::ControlFlow};

use datafusion::{
    common::{DataFusionError, Result},
    sql::sqlparser::{
        ast::{
            Expr, ExprWithAliasAndOrderBy, Function, Ident, PipeOperator, Query, Spanned,
            Statement, Visit, VisitMut, Visitor, VisitorMut,
        },
        tokenizer::Span,
    },
};

use super::{
    arguments::function_name,
    chart::{contains_mark, group_name, is_mark, named, normalized},
};
use crate::{error::at_with_help, registry::Registry};

/// Checks the places of the mark calls of `statement`, and names the items that hold them.
pub(crate) fn check(registry: &dyn Registry, statement: &mut Statement) -> Result<()> {
    match VisitMut::visit(statement, &mut Places { registry }) {
        ControlFlow::Break(error) => Err(error),
        ControlFlow::Continue(()) => Ok(()),
    }
}

/// The check of each query of a statement.
struct Places<'a> {
    registry: &'a dyn Registry,
}

impl VisitorMut for Places<'_> {
    type Break = DataFusionError;

    fn pre_visit_query(&mut self, query: &mut Query) -> ControlFlow<Self::Break> {
        match check_query(query, self.registry) {
            Ok(()) => ControlFlow::Continue(()),
            Err(error) => ControlFlow::Break(error),
        }
    }
}

/// Checks the places of the mark calls of `query`, outside its subqueries, which the visit checks
/// as queries of their own, and names the items that hold them.
fn check_query(query: &mut Query, registry: &dyn Registry) -> Result<()> {
    if let Some(mark) = first_mark(query.body.as_ref()) {
        return Err(elsewhere(&mark));
    }
    for operator in &mut query.pipe_operators {
        let PipeOperator::Aggregate {
            full_table_exprs: items,
            group_by_expr: groups,
        } = operator
        else {
            if let Some(mark) = first_mark(&*operator) {
                return Err(elsewhere(&mark));
            }
            continue;
        };
        for group in groups.iter() {
            if let Some(mark) = first_mark(&group.expr.expr) {
                return Err(at_with_help(
                    mark.name.span(),
                    format!("a group of an AGGREGATE is not a chart, so it takes no `{}`", mark.name),
                    "draw the chart in an item of the AGGREGATE, such as `|> AGGREGATE bar(x => species, y => count(*)) AS bars GROUP BY island`",
                ));
            }
        }
        for item in items.iter() {
            check_item(&item.expr.expr, registry)?;
        }
        if (items.iter()).any(|item| contains_mark(&item.expr.expr)) {
            name_items(items, groups)?;
        }
    }
    Ok(())
}

/// The error of a mark call outside an item of a pipe `AGGREGATE`.
fn elsewhere(mark: &Function) -> DataFusionError {
    at_with_help(
        mark.name.span(),
        format!(
            "`{}` builds a chart of the rows of a group, so it is valid only in an item of |> AGGREGATE",
            mark.name
        ),
        format!(
            "draw it in an AGGREGATE, such as `|> AGGREGATE {}(...) AS chart GROUP BY <groups>`, then read its column",
            mark.name
        ),
    )
}

/// The first mark call of `node`, outside its subqueries.
fn first_mark<T: Visit>(node: &T) -> Option<Function> {
    struct Find {
        queries: usize,
    }
    impl Visitor for Find {
        type Break = Function;
        fn pre_visit_query(&mut self, _query: &Query) -> ControlFlow<Function> {
            self.queries += 1;
            ControlFlow::Continue(())
        }
        fn post_visit_query(&mut self, _query: &Query) -> ControlFlow<Function> {
            self.queries -= 1;
            ControlFlow::Continue(())
        }
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Function> {
            match expr {
                Expr::Function(function) if self.queries == 0 && is_mark(expr) => {
                    ControlFlow::Break(function.clone())
                }
                _ => ControlFlow::Continue(()),
            }
        }
    }
    match Visit::visit(node, &mut Find { queries: 0 }) {
        ControlFlow::Break(function) => Some(function),
        ControlFlow::Continue(()) => None,
    }
}

/// Checks the mark calls of an item of a pipe `AGGREGATE`: none has `OVER`, and none is in the
/// arguments of an aggregate, a window function, or another mark call.
fn check_item(expr: &Expr, registry: &dyn Registry) -> Result<()> {
    struct Check<'a> {
        registry: &'a dyn Registry,
        queries: usize,
        /// For each expression that the visit is in, the name of its call when it is an aggregate
        /// or a window function.
        around: Vec<Option<Ident>>,
    }
    impl Visitor for Check<'_> {
        type Break = DataFusionError;
        fn pre_visit_query(&mut self, _query: &Query) -> ControlFlow<Self::Break> {
            self.queries += 1;
            ControlFlow::Continue(())
        }
        fn post_visit_query(&mut self, _query: &Query) -> ControlFlow<Self::Break> {
            self.queries -= 1;
            ControlFlow::Continue(())
        }
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
            let Expr::Function(function) = expr else {
                self.around.push(None);
                return ControlFlow::Continue(());
            };
            if self.queries == 0 && is_mark(expr) {
                if function.over.is_some() {
                    return ControlFlow::Break(at_with_help(
                        function.name.span(),
                        format!("a mark call takes no OVER clause, so `{}` has none", function.name),
                        "draw a chart for each group with GROUP BY, such as `|> AGGREGATE bar(x => species, y => count(*)) AS bars GROUP BY island`",
                    ));
                }
                if let Some(outer) = self.around.iter().rev().flatten().next() {
                    return ControlFlow::Break(at_with_help(
                        function.name.span(),
                        format!(
                            "`{outer}` reads the values of the rows, so it takes no mark call `{}`",
                            function.name
                        ),
                        "build the charts in one AGGREGATE, then compose them in the next, such as `|> AGGREGATE layer_agg(chart)`",
                    ));
                }
            }
            let aggregate = function.over.is_some()
                || function_name(function).is_some_and(|name| self.registry.is_aggregate(&name));
            self.around.push(
                aggregate
                    .then(|| Ident::with_span(function.name.span(), function.name.to_string())),
            );
            ControlFlow::Continue(())
        }
        fn post_visit_expr(&mut self, _expr: &Expr) -> ControlFlow<Self::Break> {
            self.around.pop();
            ControlFlow::Continue(())
        }
    }
    let mut check = Check {
        registry,
        queries: 0,
        around: Vec::new(),
    };
    match Visit::visit(expr, &mut check) {
        ControlFlow::Break(error) => Err(error),
        ControlFlow::Continue(()) => Ok(()),
    }
}

/// Names each item of `items` without an alias that holds a mark call by the name of its function,
/// and checks that each column of the output, an item or a group, has a name of its own. The
/// output holds the columns without the qualifiers of the input, so `GROUP BY a.island, b.island`
/// names two columns `island`.
fn name_items(
    items: &mut [ExprWithAliasAndOrderBy],
    groups: &[ExprWithAliasAndOrderBy],
) -> Result<()> {
    for item in items.iter_mut() {
        let expr = &item.expr.expr;
        if item.expr.alias.is_some() || !contains_mark(expr) {
            continue;
        }
        let Expr::Function(function) = expr else {
            return Err(at_with_help(
                expr.span(),
                "this item holds a mark call, so it needs a name",
                "name it with AS, such as `CASE WHEN count(*) > 100 THEN bar(...) ELSE symbol(...) END AS chart`",
            ));
        };
        let name = function_name(function).unwrap_or_else(|| function.name.to_string());
        item.expr.alias = Some(Ident::with_span(function.name.span(), name.to_lowercase()));
    }
    let items = (items.iter()).map(|item| {
        let name = named(&item.expr.expr, &item.expr.alias);
        (name.value, name.span)
    });
    let groups = (groups.iter()).map(|group| {
        let name = match (&group.expr.alias, &group.expr.expr) {
            (Some(alias), _) => normalized(alias),
            (None, expr @ (Expr::Identifier(_) | Expr::CompoundIdentifier(_))) => group_name(expr),
            (None, expr) => named(expr, &None),
        };
        let span: Span = group.expr.expr.span();
        (name.value, span)
    });
    let mut seen = HashSet::new();
    for (name, span) in items.chain(groups) {
        if !seen.insert(name.clone()) {
            return Err(at_with_help(
                span,
                format!("`{name}` names two columns of this AGGREGATE"),
                "give one of them a name of its own with AS, such as `bar(...) AS bars` or `GROUP BY a.island, b.island AS other_island`; an item of a mark call without a name takes the name of its function",
            ));
        }
    }
    Ok(())
}
