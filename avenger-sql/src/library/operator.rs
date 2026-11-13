//! The operators, as forms under the names that GoogleSQL's resolved tree gives them, such as
//! `$add` for `+` and `$equal` for `=`. The pass resolves an operator as it resolves a call, so
//! `5 / 2` is `$divide(FLOAT64, FLOAT64)` and 2.5, and `DATE '2024-01-01' = '2024-01-01'` is
//! `$equal(DATE, DATE)`. The library keeps the operators apart from its functions, so completion
//! and the reference don't list them.
//!
//! `||` is no operator here: GoogleSQL resolves it to `CONCAT` or `ARRAY_CONCAT`, and so does the
//! pass. Where DataFusion differs from GoogleSQL:
//!
//! - Integer arithmetic wraps on overflow, and a float divided by 0 is infinity or `NaN`, where
//!   GoogleSQL fails.
//! - A `NUMERIC` product or quotient is computed as a `BIGNUMERIC` of 9 decimal places, and a
//!   `BIGNUMERIC` one with fewer decimal places than 38, so its last digits can differ.
//! - `BYTES LIKE BYTES` matches the bytes as UTF-8 text, and fails for bytes that aren't UTF-8.
//! - `x IN UNNEST(array)` is `NULL` for an array that holds `NULL` and not `x` only when `x` is
//!   `NULL`.
//! - The bitwise operators of `BYTES`, and `INTERVAL * FLOAT64`, are errors.

use datafusion::{
    arrow::datatypes::{DataType, IntervalUnit},
    functions_nested::expr_fn as nested,
    logical_expr::{binary_expr, when, Expr, Operator},
};

use super::{cast, interval, lit_i64, null, CallError, Function, Ty};
use crate::types;

/// The numbers of arithmetic, in GoogleSQL's order of preference.
const NUMBERS: &str = "INT64 | NUMERIC | BIGNUMERIC | FLOAT64";

pub(super) fn operators() -> Vec<Function> {
    vec![
        add(),
        subtract(),
        multiply(),
        divide(),
        Function::scalar("$unary_minus", "-x").form(
            &format!("(x: T) -> T where T: {NUMBERS} | INTERVAL"),
            |call| Ok(Expr::Negative(Box::new(call.arg(0)))),
        ),
        comparison("$equal", Operator::Eq),
        comparison("$not_equal", Operator::NotEq),
        comparison("$less", Operator::Lt),
        comparison("$less_or_equal", Operator::LtEq),
        comparison("$greater", Operator::Gt),
        comparison("$greater_or_equal", Operator::GtEq),
        Function::scalar("$and", "x AND y").form("(x: BOOL, y: BOOL) -> BOOL", |call| {
            Ok(call.arg(0).and(call.arg(1)))
        }),
        Function::scalar("$or", "x OR y").form("(x: BOOL, y: BOOL) -> BOOL", |call| {
            Ok(call.arg(0).or(call.arg(1)))
        }),
        Function::scalar("$not", "NOT x").form("(x: BOOL) -> BOOL", |call| Ok(!call.arg(0))),
        Function::scalar("$like", "x LIKE pattern")
            .form("(x: STRING, pattern: STRING) -> BOOL", |call| {
                Ok(call.arg(0).like(call.arg(1)))
            })
            .form("(x: BYTES, pattern: BYTES) -> BOOL", |call| {
                Ok(cast(call.arg(0), DataType::Utf8).like(cast(call.arg(1), DataType::Utf8)))
            }),
        quantified_like("$like_any", false, Expr::or),
        quantified_like("$like_all", false, Expr::and),
        quantified_like("$not_like_any", true, Expr::or),
        quantified_like("$not_like_all", true, Expr::and),
        like_unnest("$like_any_unnest", false, false),
        like_unnest("$like_all_unnest", true, false),
        like_unnest("$not_like_any_unnest", false, true),
        like_unnest("$not_like_all_unnest", true, true),
        Function::scalar("$between", "x BETWEEN low AND high")
            .form("(x: T, low: T, high: T) -> BOOL where T: ANY", |call| {
                Ok(call.arg(0).between(call.arg(1), call.arg(2)))
            }),
        Function::scalar("$in", "x IN (values)")
            .form("(x: T, values: T...) -> BOOL where T: ANY", |call| {
                Ok(call.arg(0).in_list(call.rest(), false))
            }),
        Function::scalar("$in_array", "x IN UNNEST(array)").form(
            "(x: T, array: ARRAY<T>) -> BOOL where T: ANY",
            |call| {
                let (x, array) = (call.arg(0), call.arg(1));
                Ok(
                    when(x.clone().is_null().or(array.clone().is_null()), null())
                        .otherwise(nested::array_has(array, x))?,
                )
            },
        ),
        Function::scalar("$is_null", "x IS NULL")
            .form("(x: ANY) -> BOOL", |call| Ok(call.arg(0).is_null())),
        Function::scalar("$is_true", "x IS TRUE")
            .form("(x: BOOL) -> BOOL", |call| Ok(call.arg(0).is_true())),
        Function::scalar("$is_false", "x IS FALSE")
            .form("(x: BOOL) -> BOOL", |call| Ok(call.arg(0).is_false())),
        Function::scalar("$is_distinct_from", "x IS DISTINCT FROM y").form(
            "(x: T, y: T) -> BOOL where T: ANY",
            |call| {
                Ok(binary_expr(
                    call.arg(0),
                    Operator::IsDistinctFrom,
                    call.arg(1),
                ))
            },
        ),
        Function::scalar("$is_not_distinct_from", "x IS NOT DISTINCT FROM y").form(
            "(x: T, y: T) -> BOOL where T: ANY",
            |call| {
                Ok(binary_expr(
                    call.arg(0),
                    Operator::IsNotDistinctFrom,
                    call.arg(1),
                ))
            },
        ),
        bitwise("$bitwise_and", Operator::BitwiseAnd, "&"),
        bitwise("$bitwise_or", Operator::BitwiseOr, "|"),
        bitwise("$bitwise_xor", Operator::BitwiseXor, "^"),
        shift("$bitwise_left_shift", Operator::BitwiseShiftLeft, "<<"),
        shift("$bitwise_right_shift", Operator::BitwiseShiftRight, ">>"),
        Function::scalar("$bitwise_not", "~x")
            .form("(x: INT64) -> INT64", |call| {
                Ok(binary_expr(call.arg(0), Operator::BitwiseXor, lit_i64(-1)))
            })
            .form("(x: BYTES) -> BYTES", |call| {
                Ok(super::bytes::bytes_bitwise("~", call.arg(0), None))
            }),
    ]
}

/// The operators whose operands GoogleSQL rejects as a literal `NULL`, with the text of each.
pub(crate) fn rejects_null(name: &str) -> Option<&'static str> {
    let text = match name {
        "$add" => "+",
        "$subtract" | "$unary_minus" => "-",
        "$multiply" => "*",
        "$divide" => "/",
        "$equal" => "=",
        "$not_equal" => "!=",
        "$less" => "<",
        "$less_or_equal" => "<=",
        "$greater" => ">",
        "$greater_or_equal" => ">=",
        "$not" => "NOT",
        "$like"
        | "$like_any"
        | "$like_all"
        | "$not_like_any"
        | "$not_like_all"
        | "$like_any_unnest"
        | "$like_all_unnest"
        | "$not_like_any_unnest"
        | "$not_like_all_unnest" => "LIKE",
        "$bitwise_and" => "&",
        "$bitwise_or" => "|",
        "$bitwise_xor" => "^",
        "$bitwise_not" => "~",
        _ => return None,
    };
    Some(text)
}

/// `x op y` of numbers, which computes a `NUMERIC` or a `BIGNUMERIC` product or quotient at a
/// wider type, so Arrow's decimal arithmetic doesn't overflow at the scale of its result.
fn numbers(op: Operator) -> impl Fn(&super::Call) -> Result<Expr, CallError> + Send + Sync {
    move |call| {
        let (x, y) = (call.arg(0), call.arg(1));
        let wide = match (call.var("T"), op) {
            (Ty::Numeric, Operator::Multiply | Operator::Divide) => {
                Some(DataType::Decimal256(76, 9))
            }
            (Ty::BigNumeric, Operator::Multiply) => Some(DataType::Decimal256(76, 19)),
            (Ty::BigNumeric, Operator::Divide) => Some(DataType::Decimal256(76, 16)),
            _ => None,
        };
        Ok(match wide {
            Some(ty) => binary_expr(cast(x, ty.clone()), op, cast(y, ty)),
            None => binary_expr(x, op, y),
        })
    }
}

fn add() -> Function {
    Function::scalar("$add", "x + y")
        .form(
            &format!("(x: T, y: T) -> T where T: {NUMBERS}"),
            numbers(Operator::Plus),
        )
        .form("(x: DATE, y: INT64) -> DATE", |call| {
            Ok(add_days(call.arg(0), call.arg(1)))
        })
        .form("(x: INT64, y: DATE) -> DATE", |call| {
            Ok(add_days(call.arg(1), call.arg(0)))
        })
        .form("(x: DATE, y: INTERVAL) -> DATETIME", |call| {
            Ok(cast(call.arg(0), types::datetime()) + call.arg(1))
        })
        .form("(x: INTERVAL, y: DATE) -> DATETIME", |call| {
            Ok(cast(call.arg(1), types::datetime()) + call.arg(0))
        })
        .form("(x: DATETIME, y: INTERVAL) -> DATETIME", |call| {
            Ok(call.arg(0) + call.arg(1))
        })
        .form("(x: INTERVAL, y: DATETIME) -> DATETIME", |call| {
            Ok(call.arg(1) + call.arg(0))
        })
        .form("(x: TIMESTAMP, y: INTERVAL) -> TIMESTAMP", |call| {
            Ok(call.arg(0) + call.arg(1))
        })
        .form("(x: INTERVAL, y: TIMESTAMP) -> TIMESTAMP", |call| {
            Ok(call.arg(1) + call.arg(0))
        })
        .form("(x: INTERVAL, y: INTERVAL) -> INTERVAL", |call| {
            Ok(call.arg(0) + call.arg(1))
        })
}

fn subtract() -> Function {
    Function::scalar("$subtract", "x - y")
        .form(
            &format!("(x: T, y: T) -> T where T: {NUMBERS}"),
            numbers(Operator::Minus),
        )
        .form("(x: DATE, y: INT64) -> DATE", |call| {
            Ok(add_days(call.arg(0), Expr::Negative(Box::new(call.arg(1)))))
        })
        .form("(x: DATE, y: DATE) -> INTERVAL", |call| {
            Ok(interval::of_days(call.arg(0) - call.arg(1)))
        })
        .form("(x: DATE, y: INTERVAL) -> DATETIME", |call| {
            Ok(cast(call.arg(0), types::datetime()) - call.arg(1))
        })
        .form("(x: DATETIME, y: DATETIME) -> INTERVAL", |call| {
            Ok(cast(call.arg(0) - call.arg(1), interval_type()))
        })
        .form("(x: DATETIME, y: INTERVAL) -> DATETIME", |call| {
            Ok(call.arg(0) - call.arg(1))
        })
        .form("(x: TIMESTAMP, y: TIMESTAMP) -> INTERVAL", |call| {
            Ok(cast(call.arg(0) - call.arg(1), interval_type()))
        })
        .form("(x: TIMESTAMP, y: INTERVAL) -> TIMESTAMP", |call| {
            Ok(call.arg(0) - call.arg(1))
        })
        .form("(x: TIME, y: TIME) -> INTERVAL", |call| {
            Ok(call.arg(0) - call.arg(1))
        })
        .form("(x: INTERVAL, y: INTERVAL) -> INTERVAL", |call| {
            Ok(call.arg(0) - call.arg(1))
        })
}

fn multiply() -> Function {
    Function::scalar("$multiply", "x * y")
        .form(
            &format!("(x: T, y: T) -> T where T: {NUMBERS}"),
            numbers(Operator::Multiply),
        )
        .form("(x: INTERVAL, y: INT64) -> INTERVAL", |call| {
            Ok(interval::multiply(call.arg(0), call.arg(1)))
        })
        .form("(x: INT64, y: INTERVAL) -> INTERVAL", |call| {
            Ok(interval::multiply(call.arg(1), call.arg(0)))
        })
}

fn divide() -> Function {
    Function::scalar("$divide", "x / y")
        .form(
            "(x: T, y: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC",
            numbers(Operator::Divide),
        )
        .form("(x: INTERVAL, y: INT64) -> INTERVAL", |call| {
            Ok(interval::divide(call.arg(0), call.arg(1)))
        })
}

fn comparison(name: &str, op: Operator) -> Function {
    Function::scalar(name, "x op y").form("(x: T, y: T) -> BOOL where T: ANY", move |call| {
        Ok(binary_expr(call.arg(0), op, call.arg(1)))
    })
}

/// `x [NOT] LIKE ANY (patterns)` or `x [NOT] LIKE ALL (patterns)`, which join the matches of the
/// patterns, or their misses when `not`, with `join`: `NOT LIKE ANY` holds when some pattern
/// misses, and `NOT LIKE ALL` when every pattern misses, as in GoogleSQL.
fn quantified_like(name: &str, not: bool, join: fn(Expr, Expr) -> Expr) -> Function {
    Function::scalar(name, "x [NOT] LIKE {ANY | ALL} (patterns)").form(
        "(x: STRING, patterns: STRING...) -> BOOL",
        move |call| {
            let x = call.arg(0);
            Ok(call
                .rest()
                .into_iter()
                .map(|pattern| match not {
                    true => x.clone().not_like(pattern),
                    false => x.clone().like(pattern),
                })
                .reduce(join)
                .unwrap_or_else(null))
        },
    )
}

/// `x [NOT] LIKE {ANY | ALL} UNNEST(patterns)`.
fn like_unnest(name: &str, all: bool, not: bool) -> Function {
    Function::scalar(name, "x [NOT] LIKE {ANY | ALL} UNNEST(patterns)").form(
        "(x: STRING, patterns: ARRAY<STRING>) -> BOOL",
        move |call| Ok(super::like::like_unnest(call.arg(0), call.arg(1), all, not)),
    )
}

/// A bitwise operator, which GoogleSQL applies to two values of one type, without coercion.
fn bitwise(name: &str, op: Operator, symbol: &'static str) -> Function {
    Function::scalar(name, "x op y")
        .form("(x: INT64, y: INT64) -> INT64", move |call| {
            Ok(binary_expr(call.arg(0), op, call.arg(1)))
        })
        .form("(x: BYTES, y: BYTES) -> BYTES", move |call| {
            Ok(super::bytes::bytes_bitwise(
                symbol,
                call.arg(0),
                Some(call.arg(1)),
            ))
        })
}

/// A shift, which is 0 when it shifts by 64 bits or more and fails on a negative offset, as in
/// GoogleSQL. `>>` is a logical shift, which fills the high bits with zeros, as GoogleSQL's is.
fn shift(name: &str, op: Operator, symbol: &'static str) -> Function {
    Function::scalar(name, "x op y")
        .form("(x: INT64, y: INT64) -> INT64", move |call| {
            let (x, y) = (call.arg(0), call.arg(1));
            let value = match op {
                Operator::BitwiseShiftRight => {
                    // An arithmetic shift with the bits that it copies from the sign cleared.
                    let mask = binary_expr(lit_i64(i64::MAX), op, y.clone() - lit_i64(1));
                    when(y.clone().eq(lit_i64(0)), x.clone()).otherwise(binary_expr(
                        binary_expr(x, op, y.clone()),
                        Operator::BitwiseAnd,
                        mask,
                    ))?
                }
                _ => binary_expr(x, op, y.clone()),
            };
            let value = when(y.clone().gt_eq(lit_i64(64)), lit_i64(0)).otherwise(value)?;
            Ok(super::error::error_when(
                y.lt(lit_i64(0)),
                value,
                "Bitwise shift by negative offset.",
                Vec::new(),
            ))
        })
        .form("(x: BYTES, y: INT64) -> BYTES", move |call| {
            Ok(super::bytes::bytes_bitwise(
                symbol,
                call.arg(0),
                Some(call.arg(1)),
            ))
        })
}

/// `date` plus `days`, an `INT64`, through the days since the epoch that a `DATE` holds.
fn add_days(date: Expr, days: Expr) -> Expr {
    let days = cast(cast(date, DataType::Int32), DataType::Int64) + days;
    cast(cast(days, DataType::Int32), DataType::Date32)
}

fn interval_type() -> DataType {
    DataType::Interval(IntervalUnit::MonthDayNano)
}
