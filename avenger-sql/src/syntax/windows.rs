//! Windows and `unnest` in pipe `SELECT` and `EXTEND`: DataFusion 55.2 plans a window of a pipe
//! `EXTEND` and fails at execution with `Physical plan does not support logical expression
//! WindowFunction`, so the pass writes such an operator as a `SELECT` over a derived table of the
//! operators before it. Removed by DataFusion planning windows and `unnest` in pipe `SELECT` and
//! `EXTEND`, item 5 of the DataFusion contributions list.

use std::ops::ControlFlow;

use datafusion::sql::sqlparser::ast::{
    helpers::attached_token::AttachedToken, Expr, GroupByExpr, ObjectNamePart, PipeOperator, Query,
    Select, SelectFlavor, SelectItem, SetExpr, TableFactor, TableWithJoins, Visit, Visitor,
    WildcardAdditionalOptions,
};

/// Writes each pipe `SELECT` or `EXTEND` of `query` whose items call a window function or
/// `unnest` as `SELECT <items> FROM (<prefix>)`, or `SELECT *, <items> FROM (<prefix>)`, where
/// the prefix is the query before the operator. The operators after it continue on the derived
/// table, which keeps the qualifiers of the prefix in DataFusion 55.2, and the `WITH` clause of
/// `query` stays on the outer query, so the later operators read its tables.
pub(super) fn split(query: &mut Query) {
    while let Some(index) = query.pipe_operators.iter().position(windowed) {
        let rest = query.pipe_operators.split_off(index + 1);
        let projection = match query.pipe_operators.pop() {
            Some(PipeOperator::Select { exprs }) => exprs,
            Some(PipeOperator::Extend { exprs }) => {
                let mut items = vec![SelectItem::Wildcard(WildcardAdditionalOptions::default())];
                items.extend(exprs);
                items
            }
            _ => unreachable!("the operator is a SELECT or an EXTEND"),
        };
        let with = query.with.take();
        let prefix = std::mem::replace(query, empty_query());
        *query = Query {
            with,
            body: Box::new(SetExpr::Select(Box::new(select(projection, prefix)))),
            pipe_operators: rest,
            ..empty_query()
        };
    }
}

/// Whether `operator` is a pipe `SELECT` or `EXTEND` whose items call a window function or
/// `unnest`.
fn windowed(operator: &PipeOperator) -> bool {
    let (PipeOperator::Select { exprs } | PipeOperator::Extend { exprs }) = operator else {
        return false;
    };
    exprs.visit(&mut Windowed).is_break()
}

/// Breaks at a call with `OVER` or a call of `unnest`.
struct Windowed;

impl Visitor for Windowed {
    type Break = ();

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        match expr {
            Expr::Function(function)
                if function.over.is_some()
                    || matches!(
                        function.name.0.as_slice(),
                        [ObjectNamePart::Identifier(ident)]
                            if ident.value.eq_ignore_ascii_case("unnest")
                    ) =>
            {
                ControlFlow::Break(())
            }
            _ => ControlFlow::Continue(()),
        }
    }

    fn pre_visit_query(&mut self, _query: &Query) -> ControlFlow<()> {
        // A subquery plans on its own.
        ControlFlow::Continue(())
    }
}

/// A query of no rows and no operators, which the caller fills.
fn empty_query() -> Query {
    Query {
        with: None,
        body: Box::new(SetExpr::Select(Box::new(select(Vec::new(), None)))),
        order_by: None,
        limit_clause: None,
        fetch: None,
        locks: Vec::new(),
        for_clause: None,
        settings: None,
        format_clause: None,
        pipe_operators: Vec::new(),
    }
}

/// `SELECT <projection> FROM (<from>)`, or a `SELECT` without `FROM`.
fn select(projection: Vec<SelectItem>, from: impl Into<Option<Query>>) -> Select {
    let from = from.into().map(|query| TableWithJoins {
        relation: TableFactor::Derived {
            lateral: false,
            subquery: Box::new(query),
            alias: None,
            sample: None,
        },
        joins: Vec::new(),
    });
    Select {
        select_token: AttachedToken::empty(),
        optimizer_hints: Vec::new(),
        distinct: None,
        select_modifiers: None,
        top: None,
        top_before_distinct: false,
        projection,
        exclude: None,
        into: None,
        from: from.into_iter().collect(),
        lateral_views: Vec::new(),
        prewhere: None,
        selection: None,
        connect_by: Vec::new(),
        group_by: GroupByExpr::Expressions(Vec::new(), Vec::new()),
        cluster_by: Vec::new(),
        distribute_by: Vec::new(),
        sort_by: Vec::new(),
        having: None,
        named_window: Vec::new(),
        qualify: None,
        window_before_qualify: false,
        value_table_mode: None,
        flavor: SelectFlavor::Standard,
    }
}
