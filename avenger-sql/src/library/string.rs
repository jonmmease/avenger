//! The string functions, and the regular expressions, which follow RE2's syntax as GoogleSQL's
//! do.

use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    functions::expr_fn as f,
    functions_nested::expr_fn as nested,
    logical_expr::{binary_expr, when, Expr, Operator},
};

use super::{cast, lit_i64, lit_str, null_when, Call, CallError, Function, Param};

pub(super) fn functions() -> Vec<Function> {
    vec![
        Function::scalar("CONCAT", vec![Param::repeated("values")], |call| {
            // `||` returns NULL for a NULL value, as GoogleSQL's CONCAT does, where DataFusion's
            // `concat` skips it.
            let mut values = call.rest().into_iter();
            let first = values
                .next()
                .ok_or_else(|| CallError::call("CONCAT needs a value"))?;
            Ok(values.fold(first, |a, b| binary_expr(a, Operator::StringConcat, b)))
        })
        .safe(),
        Function::scalar("LENGTH", vec![Param::required("value")], |call| {
            let value = call.arg(0);
            let length = match call.type_of(&value)? {
                DataType::Binary | DataType::BinaryView | DataType::LargeBinary => {
                    f::octet_length(value)
                }
                _ => f::character_length(value),
            };
            Ok(cast(length, DataType::Int64))
        })
        .safe(),
        Function::scalar("LOWER", vec![Param::required("value")], |call| {
            Ok(f::lower(call.arg(0)))
        })
        .safe(),
        Function::scalar("UPPER", vec![Param::required("value")], |call| {
            Ok(f::upper(call.arg(0)))
        })
        .safe(),
        substr("SUBSTR"),
        substr("SUBSTRING"),
        trim("TRIM", f::btrim),
        trim("LTRIM", f::ltrim),
        trim("RTRIM", f::rtrim),
        Function::scalar(
            "REPLACE",
            vec![
                Param::required("value"),
                Param::required("from_pattern"),
                Param::required("to_pattern"),
            ],
            |call| {
                let (value, from, to) = (call.arg(0), call.arg(1), call.arg(2));
                // GoogleSQL leaves the value as it is for an empty pattern.
                Ok(when(from.clone().eq(lit_str("")), value.clone())
                    .otherwise(f::replace(value, from, to))?)
            },
        )
        .safe(),
        Function::scalar(
            "STARTS_WITH",
            vec![Param::required("value"), Param::required("prefix")],
            |call| Ok(f::starts_with(call.arg(0), call.arg(1))),
        )
        .safe(),
        Function::scalar(
            "ENDS_WITH",
            vec![Param::required("value"), Param::required("suffix")],
            |call| Ok(f::ends_with(call.arg(0), call.arg(1))),
        )
        .safe(),
        Function::scalar(
            "STRPOS",
            vec![Param::required("value"), Param::required("search")],
            |call| Ok(cast(f::strpos(call.arg(0), call.arg(1)), DataType::Int64)),
        )
        .safe(),
        Function::scalar(
            "SPLIT",
            vec![Param::required("value"), Param::optional("delimiter")],
            |call| {
                let delimiter = call.value(1).cloned().unwrap_or_else(|| lit_str(","));
                let none = Expr::Literal(ScalarValue::Utf8(None), None);
                Ok(nested::string_to_array(call.arg(0), delimiter, none))
            },
        )
        .safe(),
        Function::scalar(
            "REGEXP_CONTAINS",
            vec![Param::required("value"), Param::required("regexp")],
            |call| {
                check_pattern(call, 1)?;
                Ok(f::regexp_like(call.arg(0), call.arg(1), None))
            },
        ),
        Function::scalar(
            "REGEXP_EXTRACT",
            vec![Param::required("value"), Param::required("regexp")],
            |call| {
                // GoogleSQL returns the match, or the match of its one capturing group.
                if let Some(groups) = check_pattern(call, 1)? {
                    if groups > 1 {
                        return Err(call.error_at(
                            1,
                            "the pattern of REGEXP_EXTRACT has at most one capturing group",
                        ));
                    }
                }
                let matched = f::regexp_match(call.arg(0), call.arg(1), None);
                Ok(nested::array_element(matched, lit_i64(1)))
            },
        ),
        Function::scalar(
            "REGEXP_REPLACE",
            vec![
                Param::required("value"),
                Param::required("regexp"),
                Param::required("replacement"),
            ],
            |call| {
                check_pattern(call, 1)?;
                // GoogleSQL replaces every match.
                Ok(f::regexp_replace(
                    call.arg(0),
                    call.arg(1),
                    call.arg(2),
                    Some(lit_str("g")),
                ))
            },
        ),
    ]
}

/// `SUBSTR(value, position [, length])`, whose position counts from 1, from the end when it is
/// negative, and is 1 when it is 0.
fn substr(name: &str) -> Function {
    Function::scalar(
        name,
        vec![
            Param::required("value"),
            Param::required("position"),
            Param::optional("length"),
        ],
        |call| {
            let value = call.arg(0);
            let position = call.arg(1);
            let size = cast(f::character_length(value.clone()), DataType::Int64);
            let start = when(position.clone().gt(lit_i64(0)), position.clone())
                .when(position.clone().eq(lit_i64(0)), lit_i64(1))
                .otherwise(f::greatest(vec![size + position + lit_i64(1), lit_i64(1)]))?;
            let Some(length) = call.value(2).cloned() else {
                return Ok(f::substr(value, start));
            };
            let result = f::substring(value, start, length.clone());
            Ok(match call.safe {
                true => null_when(length.lt(lit_i64(0)), result),
                false => result,
            })
        },
    )
    .safe()
}

fn trim(name: &str, function: fn(Vec<Expr>) -> Expr) -> Function {
    Function::scalar(
        name,
        vec![Param::required("value"), Param::optional("characters")],
        move |call| {
            let mut args = vec![call.arg(0)];
            args.extend(call.value(1).cloned());
            Ok(function(args))
        },
    )
    .safe()
}

/// The number of capturing groups of the pattern at `index`, when it is a literal, or the error
/// of a pattern that doesn't compile.
fn check_pattern(call: &Call, index: usize) -> Result<Option<usize>, CallError> {
    let Some(Expr::Literal(value, _)) = call.value(index) else {
        return Ok(None);
    };
    let Some(Some(pattern)) = value.try_as_str() else {
        return Ok(None);
    };
    match regex::Regex::new(pattern) {
        Ok(regex) => Ok(Some(regex.captures_len() - 1)),
        Err(error) => Err(call.error_at(index, format!("the pattern doesn't compile: {error}"))),
    }
}
