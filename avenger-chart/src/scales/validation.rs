//! Validation utilities for scales

use datafusion::logical_expr::Expr;

/// Check if an expression references any data columns (not just literals)
pub fn expr_references_columns(expr: &Expr) -> bool {
    match expr {
        Expr::Column(_) => true,
        Expr::Literal(..) => false,
        Expr::ScalarFunction(func) => func.args.iter().any(|arg| expr_references_columns(arg)),
        Expr::BinaryExpr(binary) => {
            expr_references_columns(&binary.left) || expr_references_columns(&binary.right)
        }
        Expr::Case(case_expr) => {
            case_expr
                .expr
                .as_ref()
                .map_or(false, |e| expr_references_columns(e.as_ref()))
                || case_expr.when_then_expr.iter().any(|(when, then)| {
                    expr_references_columns(when) || expr_references_columns(then)
                })
                || case_expr
                    .else_expr
                    .as_ref()
                    .map_or(false, |e| expr_references_columns(e.as_ref()))
        }
        Expr::Cast(cast) => expr_references_columns(&cast.expr),
        Expr::TryCast(cast) => expr_references_columns(&cast.expr),
        Expr::ScalarSubquery(subquery) => {
            // Subqueries reference data by definition
            let _ = subquery;
            true
        }
        Expr::InSubquery(in_subquery) => {
            // Check if the expression being tested references columns
            expr_references_columns(&in_subquery.expr)
        }
        Expr::Exists(_) => true, // EXISTS subqueries reference data
        Expr::Between(between) => {
            expr_references_columns(&between.expr)
                || expr_references_columns(&between.low)
                || expr_references_columns(&between.high)
        }
        Expr::Like(like) => expr_references_columns(&like.expr),
        Expr::SimilarTo(like) => expr_references_columns(&like.expr),
        Expr::InList(in_list) => {
            expr_references_columns(&in_list.expr)
                || in_list.list.iter().any(|e| expr_references_columns(e))
        }
        Expr::Negative(expr) => expr_references_columns(expr),
        Expr::Not(expr) => expr_references_columns(expr),
        Expr::IsNull(expr) => expr_references_columns(expr),
        Expr::IsNotNull(expr) => expr_references_columns(expr),
        Expr::IsTrue(expr) => expr_references_columns(expr),
        Expr::IsFalse(expr) => expr_references_columns(expr),
        Expr::IsUnknown(expr) => expr_references_columns(expr),
        Expr::IsNotTrue(expr) => expr_references_columns(expr),
        Expr::IsNotFalse(expr) => expr_references_columns(expr),
        Expr::IsNotUnknown(expr) => expr_references_columns(expr),
        Expr::Alias(alias) => expr_references_columns(&alias.expr),
        Expr::AggregateFunction(agg) => agg
            .params
            .args
            .iter()
            .any(|arg| expr_references_columns(arg)),
        Expr::WindowFunction(window) => window
            .params
            .args
            .iter()
            .any(|arg| expr_references_columns(arg)),
        #[allow(deprecated)]
        Expr::Wildcard { .. } => true,
        Expr::Unnest(unnest) => expr_references_columns(&unnest.expr),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::prelude::{cast, col, lit};

    #[test]
    fn test_expr_references_columns() {
        // Column references
        assert!(expr_references_columns(&col("x")));
        assert!(expr_references_columns(&col("category")));

        // Literals do not reference columns
        assert!(!expr_references_columns(&lit("foo")));
        assert!(!expr_references_columns(&lit(42)));
        assert!(!expr_references_columns(&lit(3.14)));

        // Expressions with columns
        assert!(expr_references_columns(&(col("x") + lit(1))));
        assert!(expr_references_columns(&(col("a") * col("b"))));

        // Complex expressions
        assert!(expr_references_columns(&col("x").gt(lit(0))));

        // Cast expressions
        assert!(expr_references_columns(&cast(
            col("x"),
            datafusion::arrow::datatypes::DataType::Float32
        )));
        assert!(!expr_references_columns(&cast(
            lit(42),
            datafusion::arrow::datatypes::DataType::Float32
        )));
    }
}
