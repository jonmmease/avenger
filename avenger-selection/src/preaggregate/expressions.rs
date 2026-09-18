use datafusion::{
    common::{DFSchema, ScalarValue},
    logical_expr::{Expr, ExprSchemable, Operator},
};

/// Materialization evaluates grouping expressions before the changing filter.
/// Immutability alone does not prove that this is safe on unselected rows.
/// Start with fields, literals, checked pixel cells, and simple histogram bins.
/// Extend this proof separately from the aggregate state recipes.
pub(super) fn safe_grouping(expr: &Expr, schema: &DFSchema) -> bool {
    match expr {
        Expr::Column(_) | Expr::Literal(..) => true,
        Expr::Alias(a) => safe_grouping(&a.expr, schema),
        Expr::TryCast(c) => safe_grouping(&c.expr, schema),
        Expr::ScalarFunction(f) if f.func.inner().is::<crate::pixels::PixelCell>() => {
            f.args.iter().all(|arg| safe_grouping(arg, schema))
        }
        Expr::BinaryExpr(b) if b.op == Operator::Divide => {
            let Expr::Literal(divisor, _) = b.right.as_ref() else {
                return false;
            };
            let positive = match divisor {
                ScalarValue::Int8(Some(v)) => *v > 0,
                ScalarValue::Int16(Some(v)) => *v > 0,
                ScalarValue::Int32(Some(v)) => *v > 0,
                ScalarValue::Int64(Some(v)) => *v > 0,
                ScalarValue::UInt8(Some(v)) => *v > 0,
                ScalarValue::UInt16(Some(v)) => *v > 0,
                ScalarValue::UInt32(Some(v)) => *v > 0,
                ScalarValue::UInt64(Some(v)) => *v > 0,
                ScalarValue::Float32(Some(v)) => *v > 0.0 && v.is_finite(),
                ScalarValue::Float64(Some(v)) => *v > 0.0 && v.is_finite(),
                _ => false,
            };
            // Same types avoid a fallible implicit cast. Positive integer
            // divisors exclude zero and the MIN / -1 overflow case.
            positive
                && b.left
                    .get_type(schema)
                    .is_ok_and(|t| t == divisor.data_type())
                && safe_grouping(&b.left, schema)
        }
        _ => false,
    }
}
