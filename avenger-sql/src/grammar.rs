//! The grammar of avenger-sql: a statement is one pipe query, and every query in it, nested
//! queries included, is `FROM <source>` and then pipe operators.
//!
//! A source is a table, a table function, such as `range(0, 121) AS t(i)`, or rows of values with
//! the names of their columns, `(VALUES (1, 2), (3, 4)) AS t(a, b)`. A join target of `|> JOIN` is
//! a source or a pipe query in parentheses. A query takes no `WITH`, no `SELECT` before its
//! `FROM`, no join or clause in its `FROM`, and no `ORDER BY` or `LIMIT` outside a pipe operator.
//! The check reads the text of the user, before the syntax pass, so the queries that the passes
//! generate are standard SQL.

use std::{collections::HashSet, ops::ControlFlow};

use datafusion::{
    common::{DataFusionError, Result},
    sql::sqlparser::{
        ast::{
            GroupByExpr, PipeOperator, Query, Select, SelectFlavor, SelectItem, SetExpr, Spanned,
            Statement, TableFactor, Visit, VisitMut, Visitor, VisitorMut,
        },
        tokenizer::Span,
    },
};

use crate::error::at_with_help;

/// The help of a query that does not start with `FROM`.
const START: &str = "start the query with FROM <table>, then add its operators with |>, such as `FROM penguins |> WHERE body_mass_g > 4000 |> SELECT species`";

/// Checks that `statement` follows the grammar of avenger-sql.
pub(crate) fn check(statement: &Statement) -> Result<()> {
    let Statement::Query(query) = statement else {
        return Err(at_with_help(
            statement.span(),
            "avenger-sql reads a pipe query, not this statement",
            START,
        ));
    };
    let mut grammar = Grammar {
        values: HashSet::new(),
    };
    match Visit::visit(query.as_ref(), &mut grammar) {
        ControlFlow::Break(error) => Err(error),
        ControlFlow::Continue(()) => Ok(()),
    }
}

/// `node` with each `SELECT *` that the token pass inserts written as the `FROM`-first query of
/// the text, so that printed SQL starts its queries in parentheses with `FROM`, as the text does.
pub(crate) fn as_written<T: VisitMut>(node: &mut T) {
    struct Restore;
    impl VisitorMut for Restore {
        type Break = ();
        fn pre_visit_query(&mut self, query: &mut Query) -> ControlFlow<()> {
            if let SetExpr::Select(select) = query.body.as_mut() {
                if inserted(select) && from_first(select) {
                    select.projection.clear();
                    select.flavor = SelectFlavor::FromFirstNoSelect;
                }
            }
            ControlFlow::Continue(())
        }
    }
    let _ = VisitMut::visit(node, &mut Restore);
}

/// The check of each query, which skips the queries of the `VALUES` sources, by address.
struct Grammar {
    values: HashSet<*const Query>,
}

impl Visitor for Grammar {
    type Break = DataFusionError;

    fn pre_visit_table_factor(&mut self, table: &TableFactor) -> ControlFlow<Self::Break> {
        if let TableFactor::Derived { subquery, .. } = table {
            if is_values(subquery) {
                self.values.insert(subquery.as_ref() as *const Query);
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        if self.values.contains(&(query as *const Query)) {
            return ControlFlow::Continue(());
        }
        match check_query(query) {
            Ok(()) => ControlFlow::Continue(()),
            Err(error) => ControlFlow::Break(error),
        }
    }
}

/// Whether `select` is `FROM <sources>` and nothing else: a query that starts with `FROM`, or the
/// `SELECT * FROM` of a query in parentheses that starts with `FROM`, whose `SELECT *` the token
/// pass inserts.
fn from_first(select: &Select) -> bool {
    select.flavor == SelectFlavor::FromFirstNoSelect
        || (inserted(select)
            && select.distinct.is_none()
            && select.select_modifiers.is_none()
            && select.top.is_none()
            && select.exclude.is_none()
            && select.into.is_none()
            && select.lateral_views.is_empty()
            && select.prewhere.is_none()
            && select.selection.is_none()
            && select.connect_by.is_empty()
            && matches!(&select.group_by, GroupByExpr::Expressions(items, modifiers) if items.is_empty() && modifiers.is_empty())
            && select.cluster_by.is_empty()
            && select.distribute_by.is_empty()
            && select.sort_by.is_empty()
            && select.having.is_none()
            && select.named_window.is_empty()
            && select.qualify.is_none()
            && select.value_table_mode.is_none())
}

/// Whether the token pass inserted the `SELECT *` of `select`, whose token has an empty span.
fn inserted(select: &Select) -> bool {
    select.select_token.0.span == Span::empty()
        && matches!(select.projection.as_slice(), [SelectItem::Wildcard(_)])
}

/// Whether `query` is rows of values and nothing else.
fn is_values(query: &Query) -> bool {
    matches!(query.body.as_ref(), SetExpr::Values(_)) && plain(query)
}

/// Whether `query` has no clause around its body.
fn plain(query: &Query) -> bool {
    query.with.is_none()
        && query.order_by.is_none()
        && query.limit_clause.is_none()
        && query.fetch.is_none()
        && query.locks.is_empty()
        && query.for_clause.is_none()
        && query.settings.is_none()
        && query.format_clause.is_none()
        && query.pipe_operators.is_empty()
}

/// Checks one query: `FROM <source>`, then pipe operators, whose join targets are sources or pipe
/// queries.
fn check_query(query: &Query) -> Result<()> {
    if let Some(with) = &query.with {
        return Err(at_with_help(
            with.with_token.0.span,
            "avenger-sql takes no WITH",
            "write each subquery where the query reads it, such as `|> JOIN (FROM t |> ...) AS s ON ...`",
        ));
    }
    match query.body.as_ref() {
        SetExpr::Select(select) if from_first(select) => {
            if let Some(join) = (select.from.iter().flat_map(|from| &from.joins)).next() {
                return Err(at_with_help(
                    join.span(),
                    "the FROM of a query reads one source",
                    "join it with the |> JOIN operator, such as `FROM a |> JOIN b ON a.k = b.k`",
                ));
            }
            if let Some(second) = select.from.get(1) {
                return Err(at_with_help(
                    second.relation.span(),
                    "the FROM of a query reads one source",
                    "join it with the |> CROSS JOIN operator",
                ));
            }
            check_source(&select.from[0].relation, Position::From)?;
        }
        SetExpr::Select(select) if inserted(select) => {
            return Err(at_with_help(
                select.span(),
                "the FROM of a query in parentheses reads one source, with no other clause",
                START,
            ));
        }
        SetExpr::Select(select) => {
            return Err(at_with_help(
                select.select_token.0.span,
                "a query of avenger-sql starts with FROM, not SELECT",
                "write `SELECT <items> FROM t WHERE <condition>` as `FROM t |> WHERE <condition> |> SELECT <items>`",
            ));
        }
        SetExpr::Values(values) => {
            return Err(at_with_help(
                values.span(),
                "a query of avenger-sql starts with FROM, not VALUES",
                "read rows of values with `FROM (VALUES (1, 2), (3, 4)) AS t(a, b)`",
            ));
        }
        SetExpr::SetOperation { .. } => {
            return Err(at_with_help(
                query.body.span(),
                "a query of avenger-sql starts with FROM, not a set operation",
                "combine queries with a pipe operator, such as `FROM a |> UNION ALL (FROM b)`",
            ));
        }
        SetExpr::Query(_) => {
            return Err(at_with_help(
                query.body.span(),
                "a query of avenger-sql starts with FROM, not a query in parentheses",
                "write the query without its parentheses",
            ));
        }
        other => {
            return Err(at_with_help(
                other.span(),
                "a query of avenger-sql starts with FROM",
                START,
            ))
        }
    }
    if let Some(order_by) = &query.order_by {
        return Err(at_with_help(
            order_by.span(),
            "the ORDER BY of a query is a pipe operator",
            "order the rows with |> ORDER BY",
        ));
    }
    if let Some(limit) = &query.limit_clause {
        return Err(at_with_help(
            limit.span(),
            "the LIMIT of a query is a pipe operator",
            "limit the rows with |> LIMIT",
        ));
    }
    if !plain(&Query {
        pipe_operators: Vec::new(),
        ..query.clone()
    }) {
        return Err(at_with_help(
            query.body.span(),
            "a query of avenger-sql is FROM <source> and pipe operators, with no other clause",
            START,
        ));
    }
    for operator in &query.pipe_operators {
        if let PipeOperator::Join(join) = operator {
            check_source(&join.relation, Position::Join)?;
        }
    }
    Ok(())
}

/// Where a source is: the `FROM` of a query, or the target of a `|> JOIN`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    From,
    Join,
}

/// Checks a source: a table, a table function, `unnest`, or named rows of values, and in a join
/// target, a pipe query in parentheses, which the visit checks as a query.
fn check_source(table: &TableFactor, position: Position) -> Result<()> {
    match table {
        TableFactor::Table { .. } | TableFactor::UNNEST { .. } => Ok(()),
        TableFactor::Derived {
            lateral: false,
            subquery,
            alias,
            ..
        } if is_values(subquery) => match alias {
            Some(alias) if !alias.columns.is_empty() => Ok(()),
            _ => Err(at_with_help(
                table.span(),
                "rows of values take a name and the names of their columns",
                "name them after the parentheses, such as `(VALUES (1, 2), (3, 4)) AS t(a, b)`",
            )),
        },
        TableFactor::Derived { lateral: false, .. } if position == Position::Join => Ok(()),
        TableFactor::Derived { lateral: false, .. } => Err(at_with_help(
            table.span(),
            "the FROM of a query reads a table, a table function, or rows of values, not a query",
            "continue the query instead, so `FROM (FROM t |> WHERE c) |> SELECT a` is `FROM t |> WHERE c |> SELECT a`",
        )),
        TableFactor::NestedJoin { .. } => Err(at_with_help(
            table.span(),
            "a source of avenger-sql is not a join",
            "join with the |> JOIN operator",
        )),
        _ => Err(at_with_help(
            table.span(),
            "a source of avenger-sql is a table, a table function, or rows of values",
            "read rows of values with `(VALUES (1, 2), (3, 4)) AS t(a, b)`",
        )),
    }
}
