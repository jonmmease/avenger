//! The math functions.

use datafusion::{
    functions::expr_fn as f,
    logical_expr::{lit, Expr, Operator},
};

use super::{
    cast, float_of_integer, is_integer, lit_f64, null_if_any_null, null_when, Call, CallError,
    Function, Param,
};

pub(super) fn functions() -> Vec<Function> {
    vec![
        unary("ABS", f::abs),
        Function::scalar("SIGN", vec![Param::required("x")], |call| {
            let x = call.arg(0);
            let ty = call.type_of(&x)?;
            // DataFusion's signum returns a float, and GoogleSQL keeps an integer's type.
            Ok(match is_integer(&ty) {
                true => cast(
                    f::signum(cast(x, datafusion::arrow::datatypes::DataType::Float64)),
                    ty,
                ),
                false => f::signum(x),
            })
        })
        .safe(),
        rounding("ROUND", f::round),
        rounding("TRUNC", f::trunc),
        float_unary("FLOOR", f::floor),
        float_unary("CEIL", f::ceil),
        float_unary("CEILING", f::ceil),
        Function::scalar("SQRT", vec![Param::required("x")], |call| {
            let x = float_of_integer(call, call.arg(0))?;
            Ok(guard(call, x.clone().lt(lit_f64(0.0)), f::sqrt(x)))
        })
        .safe(),
        float_unary("EXP", f::exp),
        Function::scalar("LN", vec![Param::required("x")], |call| {
            let x = float_of_integer(call, call.arg(0))?;
            Ok(guard(call, x.clone().lt_eq(lit_f64(0.0)), f::ln(x)))
        })
        .safe(),
        // `LOG(x)` is the natural logarithm, and `LOG(x, b)` the logarithm in the base `b`.
        Function::scalar(
            "LOG",
            vec![Param::required("x"), Param::optional("base")],
            |call| {
                let x = float_of_integer(call, call.arg(0))?;
                let Some(base) = call.value(1).cloned() else {
                    return Ok(guard(call, x.clone().lt_eq(lit_f64(0.0)), f::ln(x)));
                };
                let base = float_of_integer(call, base)?;
                let invalid = x
                    .clone()
                    .lt_eq(lit_f64(0.0))
                    .or(base.clone().lt_eq(lit_f64(0.0)))
                    .or(base.clone().eq(lit_f64(1.0)));
                Ok(guard(call, invalid, f::log(base, x)))
            },
        )
        .safe(),
        Function::scalar("LOG10", vec![Param::required("x")], |call| {
            let x = float_of_integer(call, call.arg(0))?;
            Ok(guard(call, x.clone().lt_eq(lit_f64(0.0)), f::log10(x)))
        })
        .safe(),
        Function::scalar(
            "POW",
            vec![Param::required("x"), Param::required("y")],
            power,
        )
        .safe(),
        Function::scalar(
            "POWER",
            vec![Param::required("x"), Param::required("y")],
            power,
        )
        .safe(),
        // `MOD` takes the sign of its dividend, and `DIV` truncates toward zero, as DataFusion's
        // integer operators do.
        Function::scalar(
            "MOD",
            vec![Param::required("x"), Param::required("y")],
            |call| {
                let (x, y) = (call.arg(0), call.arg(1));
                Ok(divide(call, x, Operator::Modulo, y))
            },
        )
        .safe(),
        Function::scalar(
            "DIV",
            vec![Param::required("x"), Param::required("y")],
            |call| {
                let (x, y) = (call.arg(0), call.arg(1));
                for index in [0, 1] {
                    let ty = call.type_at(index)?;
                    if !is_integer(&ty)
                        && !matches!(ty, datafusion::arrow::datatypes::DataType::Decimal128(..))
                    {
                        return Err(call.error_at(index, "DIV takes INT64 or NUMERIC values"));
                    }
                }
                Ok(divide(call, x, Operator::Divide, y))
            },
        )
        .safe(),
        Function::scalar(
            "SAFE_DIVIDE",
            vec![Param::required("x"), Param::required("y")],
            |call| {
                let x = float_of_integer(call, call.arg(0))?;
                let y = float_of_integer(call, call.arg(1))?;
                Ok(x / f::nullif(y, lit_f64(0.0)))
            },
        )
        .safe(),
        Function::scalar("GREATEST", vec![Param::repeated("values")], |call| {
            let values = call.rest();
            Ok(null_if_any_null(&values, f::greatest(values.clone())))
        })
        .safe(),
        Function::scalar("LEAST", vec![Param::repeated("values")], |call| {
            let values = call.rest();
            Ok(null_if_any_null(&values, f::least(values.clone())))
        })
        .safe(),
    ]
}

/// The value of `value`, or for `SAFE.`, `NULL` where `invalid` holds and GoogleSQL fails.
fn guard(call: &Call, invalid: Expr, value: Expr) -> Expr {
    match call.safe {
        true => null_when(invalid, value),
        false => value,
    }
}

fn unary(name: &str, function: fn(Expr) -> Expr) -> Function {
    Function::scalar(name, vec![Param::required("x")], move |call| {
        Ok(function(call.arg(0)))
    })
    .safe()
}

/// A function that GoogleSQL computes in `FLOAT64` for an integer.
fn float_unary(name: &str, function: fn(Expr) -> Expr) -> Function {
    Function::scalar(name, vec![Param::required("x")], move |call| {
        Ok(function(float_of_integer(call, call.arg(0))?))
    })
    .safe()
}

/// `ROUND(x [, digits])` or `TRUNC(x [, digits])`, in `FLOAT64` for an integer.
fn rounding(name: &str, function: fn(Vec<Expr>) -> Expr) -> Function {
    Function::scalar(
        name,
        vec![Param::required("x"), Param::optional("digits")],
        move |call| {
            let mut args = vec![float_of_integer(call, call.arg(0))?];
            args.extend(call.value(1).cloned());
            Ok(function(args))
        },
    )
    .safe()
}

fn power(call: &Call) -> Result<Expr, CallError> {
    let x = cast(call.arg(0), datafusion::arrow::datatypes::DataType::Float64);
    let y = cast(call.arg(1), datafusion::arrow::datatypes::DataType::Float64);
    // GoogleSQL fails for a negative base and a fractional exponent, and for zero to a negative
    // power.
    let invalid = (x
        .clone()
        .lt(lit_f64(0.0))
        .and(f::trunc(vec![y.clone()]).not_eq(y.clone())))
    .or(x.clone().eq(lit_f64(0.0)).and(y.clone().lt(lit_f64(0.0))));
    Ok(guard(call, invalid, f::power(x, y)))
}

/// `x op y`, where `op` divides, and for `SAFE.`, `NULL` for a divisor of zero.
fn divide(call: &Call, x: Expr, op: Operator, y: Expr) -> Expr {
    let y = match call.safe {
        true => f::nullif(y, lit(0)),
        false => y,
    };
    datafusion::logical_expr::binary_expr(x, op, y)
}
