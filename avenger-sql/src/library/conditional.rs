//! The conditional functions.

use datafusion::{functions::expr_fn as f, logical_expr::when};

use super::Function;

pub(super) fn functions() -> Vec<Function> {
    vec![
        Function::scalar("IF", "Returns one of two values by a condition.")
            .description("The two results take one type, their common supertype.")
            .param("condition", "The condition. `NULL` counts as `FALSE`.")
            .param("true_result", "The result when the condition is `TRUE`.")
            .param(
                "else_result",
                "The result when the condition is `FALSE` or `NULL`.",
            )
            .form(
                "(condition: BOOL, true_result: T, else_result: T) -> T where T: ANY",
                |call| Ok(when(call.arg(0), call.arg(1)).otherwise(call.arg(2))?),
            )
            .safe()
            .example("IF(1 < 2, 'yes', 'no')", "'yes'")
            .example("IF(NULL, 1, 2)", "2"),
        Function::scalar(
            "IFNULL",
            "Returns a value, or another value when it is `NULL`.",
        )
        .param("value", "The value.")
        .param("null_result", "The result when the value is `NULL`.")
        .form("(value: T, null_result: T) -> T where T: ANY", |call| {
            Ok(f::coalesce(vec![call.arg(0), call.arg(1)]))
        })
        .safe()
        .example("IFNULL(NULL, 0)", "0")
        .example("IFNULL(5, 0)", "5"),
        Function::scalar(
            "COALESCE",
            "Returns the first of its values that is not `NULL`.",
        )
        .description("Returns `NULL` when every value is `NULL`.")
        .param("values", "The values, of one common supertype.")
        .form("(values: T...) -> T where T: ANY", |call| {
            Ok(f::coalesce(call.rest()))
        })
        .safe()
        .example("COALESCE(NULL, 'b', 'c')", "'b'"),
        Function::scalar(
            "NULLIF",
            "Returns `NULL` when two values are equal, or the first value.",
        )
        .param("value", "The value.")
        .param(
            "value_to_match",
            "The value whose equality makes the result `NULL`.",
        )
        .form("(value: T, value_to_match: T) -> T where T: ANY", |call| {
            Ok(f::nullif(call.arg(0), call.arg(1)))
        })
        .safe()
        .example("NULLIF(0, 0)", "NULL")
        .example("NULLIF(5, 0)", "5"),
    ]
}
