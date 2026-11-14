//! The math functions.

use datafusion::{
    functions::expr_fn as f,
    logical_expr::{lit, Expr, Operator},
};

use super::{error::guard, lit_f64, null_if_any_null, null_when, Call, Function, Ty};

/// The numbers that a function of floats takes, in GoogleSQL's order of preference for an
/// `INT64`, which becomes a `FLOAT64`.
const FLOATS: &str = "FLOAT64 | NUMERIC | BIGNUMERIC";
const NUMBERS: &str = "INT64 | NUMERIC | BIGNUMERIC | FLOAT64";

pub(super) fn functions() -> Vec<Function> {
    vec![
        Function::scalar("ABS", "Returns the absolute value of a number.")
            .param("x", "The number.")
            .form(&format!("(x: T) -> T where T: {NUMBERS}"), |call| {
                Ok(f::abs(call.arg(0)))
            })
            .safe()
            .example("ABS(-3)", "3")
            .example("ABS(-2.5)", "2.5"),
        Function::scalar("SIGN", "Returns the sign of a number: -1, 0, or 1.")
            .description(
                "The result has the type of the number, so the sign of a `FLOAT64` is `-1.0`, \
                 `0.0` or `1.0`.",
            )
            .param("x", "The number.")
            .form(&format!("(x: T) -> T where T: {NUMBERS}"), |call| {
                Ok(f::signum(call.arg(0)))
            })
            .safe()
            .example("SIGN(-3)", "-1")
            .example("SIGN(2.5)", "1.0"),
        rounding(
            "ROUND",
            "Rounds a number to a number of decimal places.",
            "Rounds halfway values away from zero.",
            f::round,
        )
        .example("ROUND(2.5)", "3.0")
        .example("ROUND(-2.5)", "-3.0")
        .example("ROUND(1.2345, 2)", "1.23"),
        rounding(
            "TRUNC",
            "Truncates a number toward zero, to a number of decimal places.",
            "Drops the digits after the kept places, so a negative number truncates toward zero.",
            f::trunc,
        )
        .example("TRUNC(-2.7)", "-2.0")
        .example("TRUNC(1.2345, 2)", "1.23"),
        float_unary(
            "FLOOR",
            "Returns the greatest integral value that is not greater than a number.",
            f::floor,
        )
        .example("FLOOR(1.7)", "1.0")
        .example("FLOOR(-1.2)", "-2.0"),
        float_unary(
            "CEIL",
            "Returns the least integral value that is not less than a number.",
            f::ceil,
        )
        .example("CEIL(1.2)", "2.0")
        .example("CEIL(-1.7)", "-1.0"),
        float_unary(
            "CEILING",
            "Returns the least integral value that is not less than a number, as `CEIL` does.",
            f::ceil,
        )
        .example("CEILING(1.2)", "2.0"),
        Function::scalar("SQRT", "Returns the square root of a number.")
            .param("x", "The number.")
            .form(&format!("(x: T) -> T where T: {FLOATS}"), |call| {
                let x = call.arg(0);
                let message = domain_message(
                    call,
                    &x,
                    "Argument to SQRT cannot be negative: {}",
                    "SQRT is undefined for negative value: SQRT({})",
                );
                let invalid = x.clone().lt(lit_f64(0.0));
                // DataFusion's sqrt fails on its own, so it reads `NULL` where GoogleSQL fails.
                let value = f::sqrt(null_when(invalid.clone(), x.clone()));
                Ok(guard(call, invalid, value, message, vec![x]))
            })
            .safe()
            .example("SQRT(16)", "4.0")
            .example("SAFE.SQRT(-1)", "NULL"),
        float_unary("EXP", "Returns e raised to the power of a number.", f::exp)
            .example("EXP(0)", "1.0")
            .example("EXP(1)", "2.718281828459045"),
        Function::scalar("LN", "Returns the natural logarithm of a number.")
            .param("x", "The number, which is positive.")
            .form(&format!("(x: T) -> T where T: {FLOATS}"), |call| {
                let x = call.arg(0);
                Ok(ln(call, x))
            })
            .safe()
            .example("LN(1)", "0.0")
            .example("LN(10)", "2.302585092994046"),
        Function::scalar(
            "LOG",
            "Returns the logarithm of a number, natural or in a base.",
        )
        .param("x", "The number, which is positive.")
        .param(
            "base",
            "The base of the logarithm, which is positive and not 1. When omitted, the \
             logarithm is natural, as `LN` returns.",
        )
        .form(
            &format!("(x: T[, base: T]) -> T where T: {FLOATS}"),
            |call| {
                let x = call.arg(0);
                let Some(base) = call.value(1).cloned() else {
                    return Ok(ln(call, x));
                };
                let invalid = x
                    .clone()
                    .lt_eq(lit_f64(0.0))
                    .or(base.clone().lt_eq(lit_f64(0.0)))
                    .or(base.clone().eq(lit_f64(1.0)));
                let message = domain_message(
                    call,
                    &x,
                    "Floating point error in function: LOG({}, {})",
                    "LOG is undefined for zero or negative value, or when base equals 1: LOG({}, {})",
                );
                let value = f::log(base.clone(), x.clone());
                Ok(guard(call, invalid, value, message, vec![x, base]))
            },
        )
        .safe()
        .example("LOG(100, 10)", "2.0")
        .example("LOG(8, 2)", "3.0")
        .example("LOG(100)", "4.605170185988092"),
        Function::scalar("LOG10", "Returns the base 10 logarithm of a number.")
            .param("x", "The number, which is positive.")
            .form(&format!("(x: T) -> T where T: {FLOATS}"), |call| {
                let x = call.arg(0);
                let message = domain_message(
                    call,
                    &x,
                    "Floating point error in function: LOG10({})",
                    "LOG10 is undefined for zero or negative value: LOG10({})",
                );
                let invalid = x.clone().lt_eq(lit_f64(0.0));
                Ok(guard(call, invalid, f::log10(x.clone()), message, vec![x]))
            })
            .safe()
            .example("LOG10(1000)", "3.0"),
        power("POW", "Returns a number raised to a power.")
            .example("POW(2, 10)", "1024.0")
            .example("POW(2, -1)", "0.5"),
        power(
            "POWER",
            "Returns a number raised to a power, as `POW` does.",
        )
        .example("POWER(3, 2)", "9.0"),
        Function::scalar(
            "MOD",
            "Returns the remainder of dividing one number by another.",
        )
        .description("The remainder has the sign of the dividend. A divisor of 0 is an error.")
        .param("x", "The dividend.")
        .param("y", "The divisor.")
        .form(
            "(x: T, y: T) -> T where T: INT64 | NUMERIC | BIGNUMERIC",
            |call| Ok(divide(call, call.arg(0), Operator::Modulo, call.arg(1))),
        )
        .safe()
        .example("MOD(7, 3)", "1")
        .example("MOD(-7, 3)", "-1"),
        Function::scalar(
            "DIV",
            "Divides one number by another, and truncates the quotient toward zero.",
        )
        .description("A divisor of 0 is an error.")
        .param("x", "The dividend.")
        .param("y", "The divisor.")
        .form(
            "(x: T, y: T) -> T where T: INT64 | NUMERIC | BIGNUMERIC",
            |call| {
                let quotient = divide(call, call.arg(0), Operator::Divide, call.arg(1));
                // Integers divide toward zero, and a decimal quotient truncates.
                Ok(match call.var("T") {
                    Ty::Int64 => quotient,
                    _ => f::trunc(vec![quotient]),
                })
            },
        )
        .safe()
        .example("DIV(7, 2)", "3")
        .example("DIV(-7, 2)", "-3"),
        Function::scalar(
            "SAFE_DIVIDE",
            "Divides one number by another, or returns `NULL` for a divisor of 0.",
        )
        .param("x", "The dividend.")
        .param("y", "The divisor.")
        .form(&format!("(x: T, y: T) -> T where T: {FLOATS}"), |call| {
            Ok(call.arg(0) / f::nullif(call.arg(1), lit(0)))
        })
        .safe()
        .example("SAFE_DIVIDE(10, 4)", "2.5")
        .example("SAFE_DIVIDE(1, 0)", "NULL"),
        Function::scalar("GREATEST", "Returns the greatest of its values.")
            .description("Returns `NULL` when any value is `NULL`.")
            .param("values", "The values, of one type that orders.")
            .form("(values: T...) -> T where T: ANY", |call| {
                let values = call.rest();
                Ok(null_if_any_null(&values, f::greatest(values.clone())))
            })
            .safe()
            .example("GREATEST(1, 3, 2)", "3")
            .example("GREATEST(1, NULL)", "NULL"),
        Function::scalar("LEAST", "Returns the least of its values.")
            .description("Returns `NULL` when any value is `NULL`.")
            .param("values", "The values, of one type that orders.")
            .form("(values: T...) -> T where T: ANY", |call| {
                let values = call.rest();
                Ok(null_if_any_null(&values, f::least(values.clone())))
            })
            .safe()
            .example("LEAST(1, 3, 2)", "1")
            .example("LEAST('b', 'a')", "'a'"),
    ]
}

/// `LN(x)`, as `LN` and `LOG` without a base compute it.
fn ln(call: &Call, x: Expr) -> Expr {
    let message = domain_message(
        call,
        &x,
        "Floating point error in function: LN({})",
        "LN is undefined for zero or negative value: LN({})",
    );
    let invalid = x.clone().lt_eq(lit_f64(0.0));
    guard(call, invalid, f::ln(x.clone()), message, vec![x])
}

/// GoogleSQL's message for an argument out of a function's domain: `float` for floats, and
/// `decimal` for decimals.
fn domain_message(
    call: &Call,
    x: &Expr,
    float: &'static str,
    decimal: &'static str,
) -> &'static str {
    match call.type_of(x).is_ok_and(|ty| ty.is_decimal()) {
        true => decimal,
        false => float,
    }
}

/// A function of one number that GoogleSQL computes in floats, or in decimals for a decimal.
fn float_unary(name: &str, summary: &str, function: fn(Expr) -> Expr) -> Function {
    Function::scalar(name, summary)
        .param("x", "The number.")
        .form(&format!("(x: T) -> T where T: {FLOATS}"), move |call| {
            Ok(function(call.arg(0)))
        })
        .safe()
}

/// `ROUND(x [, digits])` or `TRUNC(x [, digits])`.
fn rounding(
    name: &str,
    summary: &str,
    description: &str,
    function: fn(Vec<Expr>) -> Expr,
) -> Function {
    Function::scalar(name, summary)
        .description(description)
        .param("x", "The number. An `INT64` rounds as a `FLOAT64`.")
        .param(
            "digits",
            "The number of decimal places to keep. A negative number rounds to the left of the \
             decimal point. When omitted, 0.",
        )
        .form(
            &format!("(x: T[, digits: INT64]) -> T where T: {FLOATS}"),
            move |call| {
                let mut args = vec![call.arg(0)];
                args.extend(call.value(1).cloned());
                Ok(function(args))
            },
        )
        .safe()
}

/// `POW(x, y)` or `POWER`.
fn power(name: &str, summary: &str) -> Function {
    Function::scalar(name, summary)
        .param("x", "The base.")
        .param("y", "The power.")
        .form(&format!("(x: T, y: T) -> T where T: {FLOATS}"), |call| {
            let (x, y) = (call.arg(0), call.arg(1));
            // GoogleSQL fails for a negative base and a fractional power, and for zero to a
            // negative power.
            let fractional = f::trunc(vec![y.clone()]).not_eq(y.clone());
            let negative = x.clone().lt(lit_f64(0.0)).and(fractional);
            let by_zero = x.clone().eq(lit_f64(0.0)).and(y.clone().lt(lit_f64(0.0)));
            // DataFusion's power of decimals fails on its own, so it reads `NULL` where GoogleSQL
            // fails.
            let base = null_when(negative.clone().or(by_zero.clone()), x.clone());
            let value = f::power(base, y.clone());
            let args = vec![x.clone(), y.clone()];
            if !call.type_of(&x).is_ok_and(|ty| ty.is_decimal()) {
                let message = "Floating point error in function: POW({}, {})";
                return Ok(guard(call, negative.or(by_zero), value, message, args));
            }
            // A decimal has a message of its own for each.
            let message =
                "Negative NUMERIC value cannot be raised to a fractional power: POW({}, {})";
            let value = guard(call, negative, value, message, args.clone());
            Ok(guard(
                call,
                by_zero,
                value,
                "division by zero: POW({}, {})",
                args,
            ))
        })
        .safe()
}

/// `x op y`, where `op` divides, and for `SAFE.`, `NULL` for a divisor of zero.
fn divide(call: &Call, x: Expr, op: Operator, y: Expr) -> Expr {
    let y = match call.safe {
        true => f::nullif(y, lit(0)),
        false => y,
    };
    datafusion::logical_expr::binary_expr(x, op, y)
}
