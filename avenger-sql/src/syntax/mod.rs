//! The syntax pass: rewrites of the parsed statement into a statement that the DataFusion SQL
//! planner accepts and plans as avenger-sql means it. It reads the syntax tree and the function
//! registry, and each item names the upstream change that removes it:
//!
//! - A query that starts with `FROM` selects `*`, because DataFusion 55.2 plans no columns for it.
//!   Removed by DataFusion planning FROM-first queries.
//! - A pipe `SELECT` or `EXTEND` that calls a window function or `unnest` becomes a `SELECT` over
//!   a derived table ([`windows`]).
//! - The `AGGREGATE` of a `|> CHART` becomes the `FROM`-position call of the mark planner
//!   ([`chart`]). The pass does not lower the mark call, whose channels the mark planner reads as
//!   written.
//! - A named option of `bin`, `timeunit`, `lump`, or `stack` becomes an option pair, and a call
//!   with named arguments passes `NULL` for each omitted parameter before its last named argument
//!   ([`arguments`]).

mod arguments;
pub(crate) mod build;
mod chart;
mod windows;

use std::ops::ControlFlow;

use datafusion::{
    common::{DataFusionError, Result},
    sql::sqlparser::ast::{
        Expr, FunctionArg, FunctionArgExpr, PipeOperator, Query, SelectFlavor, SetExpr, Statement,
        TableFactor, Value, VisitMut, VisitorMut,
    },
};

pub(crate) use arguments::function_name;
pub(crate) use chart::{is_marker as is_chart, read as read_chart, ChartOperator};

use crate::registry::Registry;

/// Lowers `statement` for the functions of `registry`.
pub(crate) fn lower(registry: &dyn Registry, statement: &mut Statement) -> Result<()> {
    visit(registry, statement)
}

/// Lowers `query`, a query of no statement, such as the lifting of a chart call.
pub(crate) fn lower_query(registry: &dyn Registry, query: &mut Query) -> Result<()> {
    visit(registry, query)
}

/// Lowers `expr`, such as a call of a transform that the mark planner types.
pub(crate) fn lower_expr(registry: &dyn Registry, expr: &mut Expr) -> Result<()> {
    visit(registry, expr)
}

/// Applies the pass to `node` and everything in it.
fn visit<T: VisitMut>(registry: &dyn Registry, node: &mut T) -> Result<()> {
    let mut lower = Lower {
        registry,
        mark_calls: Vec::new(),
    };
    match VisitMut::visit(node, &mut lower) {
        ControlFlow::Break(error) => Err(error),
        ControlFlow::Continue(()) => Ok(()),
    }
}

/// Splits `query` at its pipe operator `index`. It returns the prefix, the query with the
/// operators before the operator and without its `WITH` clause, and the operator, and it leaves in
/// `query` its `WITH` clause, so that the later operators read its tables, and the operators after
/// the operator, with a body that the caller fills.
fn split_prefix(query: &mut Query, index: usize) -> (Query, PipeOperator) {
    let rest = query.pipe_operators.split_off(index + 1);
    let operator = (query.pipe_operators.pop()).expect("the operator at the index");
    let with = query.with.take();
    let prefix = std::mem::replace(query, build::query(build::select(Vec::new(), None)));
    query.with = with;
    query.pipe_operators = rest;
    (prefix, operator)
}

/// The items of the pass, on each query and each function call of a statement.
struct Lower<'a> {
    registry: &'a dyn Registry,
    /// The mark calls of the chart calls that the visit is in, which it keeps out of the visit.
    mark_calls: Vec<FunctionArg>,
}

impl VisitorMut for Lower<'_> {
    type Break = DataFusionError;

    fn pre_visit_query(&mut self, query: &mut Query) -> ControlFlow<DataFusionError> {
        while let Some(index) = (query.pipe_operators.iter())
            .position(|operator| windows::windowed(operator) || chart::is_marker(operator))
        {
            match chart::is_marker(&query.pipe_operators[index]) {
                true => {
                    if let Err(error) = chart::split(query, index) {
                        return ControlFlow::Break(error);
                    }
                }
                false => windows::split(query, index),
            }
        }
        if let SetExpr::Select(select) = query.body.as_mut() {
            if select.flavor == SelectFlavor::FromFirstNoSelect {
                select.projection = vec![build::wildcard()];
                select.flavor = SelectFlavor::Standard;
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, table: &mut TableFactor) -> ControlFlow<DataFusionError> {
        if let Some(call) = chart::mark_call(table) {
            let placeholder = FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::value(Value::Null)));
            self.mark_calls.push(std::mem::replace(call, placeholder));
        }
        ControlFlow::Continue(())
    }

    fn post_visit_table_factor(&mut self, table: &mut TableFactor) -> ControlFlow<DataFusionError> {
        if let Some(call) = chart::mark_call(table) {
            *call = self
                .mark_calls
                .pop()
                .expect("the mark call of the chart call");
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<DataFusionError> {
        if let Expr::Function(function) = expr {
            arguments::option_pairs(function);
            arguments::fill_omitted(function, &|name| self.registry.parameter_names(name));
        }
        ControlFlow::Continue(())
    }
}
