//! Windows and `unnest` in pipe `SELECT` and `EXTEND`: DataFusion 55.2 plans a window of a pipe
//! `EXTEND` and fails at execution with `Physical plan does not support logical expression
//! WindowFunction`, so the pass writes such an operator as a `SELECT` over a derived table of the
//! operators before it. Removed by DataFusion planning windows and `unnest` in pipe `SELECT` and
//! `EXTEND`, item 5 of the DataFusion contributions list.

use std::ops::ControlFlow;

use datafusion::sql::sqlparser::ast::{
    Expr, ObjectNamePart, PipeOperator, Query, SetExpr, Visit, Visitor,
};

use super::{build, split_prefix};

/// Whether `operator` is a pipe `SELECT` or `EXTEND` whose items call a window function or
/// `unnest`.
pub(super) fn windowed(operator: &PipeOperator) -> bool {
    let (PipeOperator::Select { exprs } | PipeOperator::Extend { exprs }) = operator else {
        return false;
    };
    exprs.visit(&mut Windowed).is_break()
}

/// Writes the pipe `SELECT` or `EXTEND` at `index` of `query` as `SELECT <items> FROM (<prefix>)`,
/// or `SELECT *, <items> FROM (<prefix>)`, where the prefix is the query before the operator. The
/// derived table keeps the qualifiers of the prefix in DataFusion 55.2.
pub(super) fn split(query: &mut Query, index: usize) {
    let (prefix, operator) = split_prefix(query, index);
    let projection = match operator {
        PipeOperator::Select { exprs } => exprs,
        PipeOperator::Extend { exprs } => std::iter::once(build::wildcard()).chain(exprs).collect(),
        _ => unreachable!("the operator is a SELECT or an EXTEND"),
    };
    *query.body = SetExpr::Select(Box::new(build::select(
        projection,
        Some(build::derived(prefix)),
    )));
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
}
