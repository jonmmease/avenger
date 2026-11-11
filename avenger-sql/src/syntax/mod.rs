//! The syntax pass: rewrites of the parsed statement into a statement that the DataFusion SQL
//! planner accepts and plans as avenger-sql means it. It reads the syntax tree and the function
//! registry, and each item names the upstream change that removes it:
//!
//! - A query that starts with `FROM` selects `*`, because DataFusion 55.2 plans no columns for it.
//!   Removed by DataFusion planning FROM-first queries.
//! - A pipe `SELECT` or `EXTEND` that calls a window function or `unnest` becomes a `SELECT` over
//!   a derived table ([`windows`]).
//! - A named option of `bin`, `timeunit`, `lump`, or `stack` becomes an option pair, and a call
//!   with named arguments passes `NULL` for each omitted parameter before its last named argument
//!   ([`arguments`]).

mod arguments;
mod windows;

use std::ops::ControlFlow;

use datafusion::{
    common::{DataFusionError, Result},
    execution::SessionState,
    sql::sqlparser::ast::{
        Expr, Query, SelectFlavor, SelectItem, SetExpr, Statement, VisitMut, VisitorMut,
        WildcardAdditionalOptions,
    },
};

/// Lowers `statement` for the functions of `state`.
pub(crate) fn lower(state: &SessionState, statement: &mut Statement) -> Result<()> {
    visit(state, statement)
}

/// Applies the pass to `node` and everything in it.
fn visit<T: VisitMut>(state: &SessionState, node: &mut T) -> Result<()> {
    let parameters = |name: &str| parameter_names(state, name);
    match VisitMut::visit(node, &mut Lower { parameters }) {
        ControlFlow::Break(error) => Err(error),
        ControlFlow::Continue(()) => Ok(()),
    }
}

/// The parameter names of the function `name` of `state`, a scalar, an aggregate, or a window
/// function, when its signature declares them.
fn parameter_names(state: &SessionState, name: &str) -> Option<Vec<String>> {
    let signature = if let Some(function) = state.scalar_functions().get(name) {
        function.signature()
    } else if let Some(function) = state.aggregate_functions().get(name) {
        function.signature()
    } else {
        state.window_functions().get(name)?.signature()
    };
    signature.parameter_names.clone()
}

/// The items of the pass, on each query and each function call of a statement.
struct Lower<F> {
    parameters: F,
}

impl<F: Fn(&str) -> Option<Vec<String>>> VisitorMut for Lower<F> {
    type Break = DataFusionError;

    fn pre_visit_query(&mut self, query: &mut Query) -> ControlFlow<DataFusionError> {
        windows::split(query);
        if let SetExpr::Select(select) = query.body.as_mut() {
            if select.flavor == SelectFlavor::FromFirstNoSelect {
                select.projection =
                    vec![SelectItem::Wildcard(WildcardAdditionalOptions::default())];
                select.flavor = SelectFlavor::Standard;
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<DataFusionError> {
        if let Expr::Function(function) = expr {
            arguments::option_pairs(function);
            arguments::fill_omitted(function, &self.parameters);
        }
        ControlFlow::Continue(())
    }
}
