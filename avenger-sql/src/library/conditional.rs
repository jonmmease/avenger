//! The conditional functions.

use datafusion::{functions::expr_fn as f, logical_expr::when};

use super::{Function, Param};

pub(super) fn functions() -> Vec<Function> {
    vec![
        Function::scalar(
            "IF",
            vec![
                Param::required("condition"),
                Param::required("true_result"),
                Param::required("else_result"),
            ],
            |call| Ok(when(call.arg(0), call.arg(1)).otherwise(call.arg(2))?),
        )
        .safe(),
        Function::scalar(
            "IFNULL",
            vec![Param::required("value"), Param::required("null_result")],
            |call| Ok(f::coalesce(vec![call.arg(0), call.arg(1)])),
        )
        .safe(),
        Function::scalar("COALESCE", vec![Param::repeated("values")], |call| {
            Ok(f::coalesce(call.rest()))
        })
        .safe(),
        Function::scalar(
            "NULLIF",
            vec![Param::required("value"), Param::required("value_to_match")],
            |call| Ok(f::nullif(call.arg(0), call.arg(1))),
        )
        .safe(),
    ]
}
