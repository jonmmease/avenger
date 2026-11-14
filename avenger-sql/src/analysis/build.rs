//! Small builders of the expressions and the plans of the pass.

use std::sync::Arc;

use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::{Cast, Expr, LogicalPlan, Subquery},
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
