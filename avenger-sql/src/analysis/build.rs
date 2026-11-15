//! Small builders of the expressions and the plans of the pass.

use std::sync::Arc;

use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::{
        expr::WindowFunction, Cast, Expr, LogicalPlan, SortExpr, Subquery, WindowFrame,
        WindowFunctionDefinition,
    },
};

/// `expr` cast to `ty`.
pub(crate) fn cast(expr: Expr, ty: DataType) -> Expr {
    Expr::Cast(Cast::new(Box::new(expr), ty))
}

/// An `INT64` literal.
pub(crate) fn int(value: i64) -> Expr {
    Expr::Literal(ScalarValue::Int64(Some(value)), None)
}

/// A `FLOAT64` literal.
pub(crate) fn float(value: f64) -> Expr {
    Expr::Literal(ScalarValue::Float64(Some(value)), None)
}

/// The subquery of `plan`, with the outer references that it reads.
pub(crate) fn subquery(plan: LogicalPlan) -> Subquery {
    Subquery {
        outer_ref_columns: plan.all_out_ref_exprs(),
        subquery: Arc::new(plan),
        spans: Default::default(),
    }
}

/// `func` of `args` over the window of `partition` in the order of `order`, whose frame runs to
/// the current row's peers when it has an order, and over the whole partition otherwise.
pub(crate) fn window(
    func: WindowFunctionDefinition,
    args: Vec<Expr>,
    partition: Vec<Expr>,
    order: Vec<SortExpr>,
) -> Expr {
    let mut window = WindowFunction::new(func, args);
    window.params.partition_by = partition;
    window.params.window_frame = WindowFrame::new((!order.is_empty()).then_some(false));
    window.params.order_by = order;
    Expr::from(window)
}
