//! The evaluation of the expressions of the tests: `parse` reads `FROM t |> WHERE <text>`, and
//! `plan` plans its literals, typed literals, `INTERVAL`, arrays, subscripts, column names, `<`,
//! `IS NOT NULL`, and calls through the library. `json` shows a value in GoogleSQL's JSON form,
//! and `literal` as a GoogleSQL literal.

#![allow(dead_code)]

use std::sync::Arc;

use avenger_sql::{
    library::{Arg, CallError, DatePart, Library, PlanContext, Planned, Subscript},
    types,
};
use avenger_sql_syntax::ast::{self, AstNode, LiteralKind};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray},
        datatypes::{
            DataType, Date32Type, Field, Float64Type, Int32Type, Int64Type, IntervalMonthDayNano,
            Schema, TimestampMicrosecondType, UInt64Type,
        },
    },
    common::{DFSchema, ScalarValue},
    functions_nested::expr_fn::make_array,
    logical_expr::{
        expr::{AggregateFunction, WindowFunction},
        lit, Cast, Expr,
    },
    prelude::SessionContext,
};

/// The schema of the tables of the tests, for the types of their columns.
pub fn schema() -> DFSchema {
    DFSchema::try_from(Schema::new(vec![
        Field::new("x", DataType::Int64, true),
        Field::new("s", DataType::Utf8, true),
    ]))
    .expect("a schema")
}

pub fn context(schema: &DFSchema) -> PlanContext<'_> {
    PlanContext {
        schema,
        time_zone: "UTC",
        type_options: Default::default(),
    }
}

/// The condition of `FROM t |> WHERE <text>`.
pub fn parse(text: &str) -> ast::Expr {
    let parse = avenger_sql_syntax::parse(&format!("FROM t |> WHERE {text}"));
    assert!(parse.ok(), "{text}: {:?}", parse.diagnostics());
    parse
        .syntax()
        .descendants()
        .find_map(ast::PipeWhere::cast)
        .and_then(|pipe| pipe.condition())
        .expect("a condition")
}

pub fn cast(expr: Expr, ty: DataType) -> Expr {
    Expr::Cast(Cast::new(Box::new(expr), ty))
}

/// The expression of `text`, planned through the library.
pub fn expr(library: &Library, text: &str) -> Expr {
    plan(library, &parse(text)).unwrap_or_else(|error| panic!("{text}: {error}"))
}

pub fn plan(library: &Library, expr: &ast::Expr) -> Result<Expr, CallError> {
    let planned = match expr {
        ast::Expr::Literal(literal) => match literal.kind().expect("a literal") {
            LiteralKind::Int => lit(literal.int_value().expect("an INT64") as i64),
            LiteralKind::Float => lit(literal.float_value().expect("a FLOAT64")),
            LiteralKind::String => lit(literal.string_value().expect("a STRING")),
            LiteralKind::Bytes => Expr::Literal(ScalarValue::Binary(literal.bytes_value()), None),
            LiteralKind::Null => Expr::Literal(ScalarValue::Null, None),
            LiteralKind::True => lit(true),
            LiteralKind::False => lit(false),
        },
        ast::Expr::TypedLiteral(typed) => {
            let value = typed.value().expect("a string");
            if typed.numeric_kw().is_some() {
                cast(lit(value), DataType::Decimal128(38, 9))
            } else if typed.date_kw().is_some() {
                cast(lit(value), DataType::Date32)
            } else if typed.datetime_kw().is_some() {
                cast(lit(value), types::datetime())
            } else {
                let value = match value.strip_suffix("+00") {
                    Some(civil) => format!("{civil}+00:00"),
                    None => value,
                };
                cast(lit(value), types::timestamp())
            }
        }
        ast::Expr::IntervalExpr(interval) => {
            let Some(ast::Expr::Literal(count)) = interval.expr() else {
                panic!("an interval of a literal");
            };
            let count = count.int_value().expect("an INT64") as i32;
            let part = interval
                .from()
                .and_then(|part| part.text())
                .expect("a part");
            let (months, days, nanos) = match part.to_ascii_uppercase().as_str() {
                "YEAR" => (12 * count, 0, 0),
                "QUARTER" => (3 * count, 0, 0),
                "MONTH" => (count, 0, 0),
                "WEEK" => (0, 7 * count, 0),
                "DAY" => (0, count, 0),
                "HOUR" => (0, 0, 3_600_000_000_000 * i64::from(count)),
                "MINUTE" => (0, 0, 60_000_000_000 * i64::from(count)),
                "SECOND" => (0, 0, 1_000_000_000 * i64::from(count)),
                other => panic!("an interval of {other}"),
            };
            Expr::Literal(
                ScalarValue::IntervalMonthDayNano(Some(IntervalMonthDayNano::new(
                    months, days, nanos,
                ))),
                None,
            )
        }
        ast::Expr::ArrayExpr(array) => make_array(
            array
                .exprs()
                .map(|element| plan(library, &element))
                .collect::<Result<_, _>>()?,
        ),
        ast::Expr::PrefixExpr(prefix) => {
            let operand = plan(library, &prefix.expr().expect("an operand"))?;
            Expr::Negative(Box::new(operand))
        }
        ast::Expr::ParenExpr(paren) => plan(library, &paren.expr().expect("an expression"))?,
        ast::Expr::BinExpr(bin) => {
            use avenger_sql_syntax::ast::BinaryOp;
            let lhs = plan(library, &bin.lhs().expect("an operand"))?;
            let rhs = plan(library, &bin.rhs().expect("an operand"))?;
            match bin.operator().expect("an operator") {
                BinaryOp::Lt => lhs.lt(rhs),
                BinaryOp::Gt => lhs.gt(rhs),
                BinaryOp::LtEq => lhs.lt_eq(rhs),
                BinaryOp::GtEq => lhs.gt_eq(rhs),
                BinaryOp::Eq => lhs.eq(rhs),
                other => panic!("the tests plan no {other:?}"),
            }
        }
        ast::Expr::IsExpr(is) => plan(library, &is.expr().expect("an operand"))?.is_not_null(),
        ast::Expr::SubscriptExpr(subscript) => {
            let base = plan(library, &subscript.base().expect("an array"))?;
            match subscript.index().expect("an index") {
                ast::Expr::CallExpr(call) => {
                    let name = call_name(&call).1;
                    let mode = Subscript::parse(&name).expect("a subscript");
                    let index = call
                        .arg_list()
                        .and_then(|list| list.arguments().next())
                        .and_then(|argument| match argument {
                            ast::Argument::Arg(arg) => arg.expr(),
                            _ => None,
                        })
                        .expect("an index");
                    mode.element(base, plan(library, &index)?)
                }
                index => Subscript::Offset.element(base, plan(library, &index)?),
            }
        }
        ast::Expr::CallExpr(call) => return plan_call(library, call),
        ast::Expr::PathExpr(path) => {
            let names = path.path().and_then(|path| path.names()).expect("a name");
            datafusion::prelude::col(names.join("."))
        }
        other => panic!("the tests plan no {other:?}"),
    };
    Ok(planned)
}

/// Whether the call writes `SAFE.`, and the name of its function.
pub fn call_name(call: &ast::CallExpr) -> (bool, String) {
    let names = call.path().and_then(|path| path.names()).expect("a name");
    match names.as_slice() {
        [safe, name] if safe.eq_ignore_ascii_case("SAFE") => (true, name.clone()),
        [name] => (false, name.clone()),
        other => panic!("a function named {other:?}"),
    }
}

pub fn plan_call(library: &Library, call: &ast::CallExpr) -> Result<Expr, CallError> {
    let (safe, name) = call_name(call);
    let function = library
        .get(&name)
        .unwrap_or_else(|| panic!("the library has no {name}"));
    let args = plan_args(library, call);
    let star = call
        .arg_list()
        .is_some_and(|list| list.star_arg().is_some());
    let schema = schema();
    match function.plan(args, star, safe, context(&schema))? {
        Planned::Scalar(expr) => Ok(expr),
        Planned::Aggregate {
            func,
            args,
            filter,
            result,
        } => Ok(typed(
            Expr::AggregateFunction(AggregateFunction::new_udf(
                func,
                args,
                false,
                filter.map(Box::new),
                vec![],
                None,
            )),
            result,
        )),
        Planned::Window { func, args, result } => {
            Ok(typed(Expr::from(WindowFunction::new(func, args)), result))
        }
    }
}

/// `expr` cast to `result`, the type of GoogleSQL's result, when the library gives one. The
/// tests cast an aggregate or a window function before they add its clauses, which the pass
/// does after.
pub fn typed(expr: Expr, result: Option<DataType>) -> Expr {
    match result {
        Some(ty) => cast(expr, ty),
        None => expr,
    }
}

/// The arguments of `call`, each planned by the kind of its parameter, and named as the call
/// names it.
pub fn plan_args(library: &Library, call: &ast::CallExpr) -> Vec<(Option<String>, Arg)> {
    let function = library.get(&call_name(call).1).expect("a function");
    let mut args = Vec::new();
    let list = call.arg_list();
    for (position, argument) in list.iter().flat_map(|list| list.arguments()).enumerate() {
        let (named, value) = match argument {
            ast::Argument::Arg(arg) => (None, arg.expr().expect("a value")),
            ast::Argument::NamedArg(arg) => (
                arg.name_ref().and_then(|name| name.text()),
                arg.expr().expect("a value"),
            ),
            other => panic!("the tests pass no {other:?}"),
        };
        let arg = match function.takes_date_part(position, named.as_deref()) {
            true => Arg::DatePart(date_part(&value)),
            false => Arg::Value(plan(library, &value).expect("an argument")),
        };
        args.push((named, arg));
    }
    args
}

/// The date part that `expr` writes, as in `MONTH` or `WEEK(MONDAY)`.
pub fn date_part(expr: &ast::Expr) -> DatePart {
    match expr {
        ast::Expr::PathExpr(path) => {
            let name = path.path().and_then(|path| path.names()).expect("a name");
            DatePart::parse(&name[0], None).expect("a date part")
        }
        ast::Expr::CallExpr(call) => {
            let day = call
                .arg_list()
                .and_then(|list| list.arguments().next())
                .map(|argument| argument.syntax().text().to_string())
                .expect("a day");
            DatePart::parse(&call_name(call).1, Some(&day)).expect("a date part")
        }
        other => panic!("{other} is not a date part"),
    }
}

/// The value of `expr` in a row of no columns.
pub async fn run(expr: Expr) -> ArrayRef {
    let ctx = SessionContext::new();
    let batches = ctx
        .read_empty()
        .expect("an empty relation")
        .select(vec![expr.alias("v")])
        .expect("a projection")
        .collect()
        .await
        .expect("a run");
    batches[0].column(0).clone()
}

/// The value at `index` of `array`, in GoogleSQL's JSON form.
pub fn json(array: &ArrayRef, index: usize) -> String {
    if array.is_null(index) {
        return "null".to_string();
    }
    match array.data_type() {
        DataType::Int64 => format!("\"{}\"", array.as_primitive::<Int64Type>().value(index)),
        DataType::Int32 => format!("\"{}\"", array.as_primitive::<Int32Type>().value(index)),
        DataType::UInt64 => format!("\"{}\"", array.as_primitive::<UInt64Type>().value(index)),
        DataType::Float64 => {
            let value = array.as_primitive::<Float64Type>().value(index);
            match value.fract() == 0.0 && value.abs() < 1e15 {
                true => format!("{}", value as i64),
                false => format!("{value}"),
            }
        }
        DataType::Boolean => array.as_boolean().value(index).to_string(),
        DataType::Utf8 => string(array.as_string::<i32>().value(index)),
        DataType::LargeUtf8 => string(array.as_string::<i64>().value(index)),
        DataType::Utf8View => string(array.as_string_view().value(index)),
        DataType::Date32 => array.as_primitive::<Date32Type>().value(index).to_string(),
        DataType::Timestamp(_, Some(_)) => {
            let micros =
                datafusion::arrow::compute::cast(&array.slice(index, 1), &types::timestamp())
                    .expect("microseconds");
            format!(
                "\"{}\"",
                micros.as_primitive::<TimestampMicrosecondType>().value(0)
            )
        }
        DataType::Timestamp(_, None) => {
            let micros =
                datafusion::arrow::compute::cast(&array.slice(index, 1), &types::datetime())
                    .expect("microseconds");
            let micros = micros.as_primitive::<TimestampMicrosecondType>().value(0);
            format!("\"{}\"", packed_datetime(micros))
        }
        DataType::List(_) | DataType::LargeList(_) => {
            let list = datafusion::arrow::compute::cast(
                &array.slice(index, 1),
                &DataType::List(Arc::new(datafusion::arrow::datatypes::Field::new(
                    "item",
                    match array.data_type() {
                        DataType::List(field) | DataType::LargeList(field) => {
                            field.data_type().clone()
                        }
                        _ => unreachable!(),
                    },
                    true,
                ))),
            )
            .expect("a list");
            let values = list.as_list::<i32>().value(0);
            let items: Vec<String> = (0..values.len()).map(|i| json(&values, i)).collect();
            format!("[{}]", items.join(", "))
        }
        // GoogleSQL's JSON writes bytes in base64.
        DataType::Binary | DataType::BinaryView | DataType::LargeBinary => {
            let bytes = datafusion::arrow::compute::cast(&array.slice(index, 1), &DataType::Binary)
                .expect("bytes");
            format!("\"{}\"", base64(bytes.as_binary::<i32>().value(0)))
        }
        other => panic!("the tests show no {other}"),
    }
}

/// `bytes` in base64, with padding.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let value = (chunk.iter().enumerate()).fold(0u32, |value, (i, byte)| {
            value | u32::from(*byte) << (16 - 8 * i)
        });
        for index in 0..4 {
            match index <= chunk.len() {
                true => out.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char),
                false => out.push('='),
            }
        }
    }
    out
}

pub fn string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The microseconds since 1970-01-01T00:00:00 of a civil time in GoogleSQL's packed form of a
/// DATETIME, whose bits hold its year, month, day, hour, minute, second and microsecond.
pub fn packed_datetime(micros: i64) -> i64 {
    let days = micros.div_euclid(86_400_000_000);
    let rest = micros.rem_euclid(86_400_000_000);
    // The civil date of a day number, from Howard Hinnant's `civil_from_days`.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (hour, minute, second, micro) = (
        rest / 3_600_000_000,
        rest / 60_000_000 % 60,
        rest / 1_000_000 % 60,
        rest % 1_000_000,
    );
    (year << 46)
        | (month << 42)
        | (day << 37)
        | (hour << 32)
        | (minute << 26)
        | (second << 20)
        | micro
}

/// The value at `index` of `array`, as a GoogleSQL literal writes it: `3`, `3.0`, `'pple'`,
/// `TRUE`, `NULL`, `DATE '2024-05-17'`, `[1, 2]`, or `STRUCT(0.0 AS start, 10.0 AS end)`.
pub fn literal(array: &ArrayRef, index: usize) -> String {
    use datafusion::arrow::util::display::array_value_to_string;
    if array.is_null(index) {
        return "NULL".to_string();
    }
    let display = || array_value_to_string(array, index).expect("a value");
    match array.data_type() {
        DataType::Int64 | DataType::Int32 | DataType::UInt64 => display(),
        DataType::Float64 => {
            let value = array.as_primitive::<Float64Type>().value(index);
            match value.fract() == 0.0 && value.abs() < 1e15 {
                true => format!("{value:.1}"),
                false => format!("{value}"),
            }
        }
        DataType::Decimal128(..) => format!(
            "NUMERIC '{}'",
            display().trim_end_matches('0').trim_end_matches('.')
        ),
        DataType::Boolean => display().to_ascii_uppercase(),
        DataType::Utf8 | DataType::Utf8View | DataType::LargeUtf8 => {
            format!("'{}'", display().replace('\\', "\\\\").replace('\'', "\\'"))
        }
        DataType::Binary | DataType::BinaryView | DataType::LargeBinary => {
            let bytes: Vec<u8> = match array.data_type() {
                DataType::Binary => array.as_binary::<i32>().value(index).to_vec(),
                DataType::LargeBinary => array.as_binary::<i64>().value(index).to_vec(),
                _ => array.as_binary_view().value(index).to_vec(),
            };
            // Printable ASCII as itself, and every other byte as `\xNN`.
            let text: String = (bytes.iter())
                .map(|byte| match byte {
                    b'\'' | b'\\' => format!("\\{}", *byte as char),
                    0x20..=0x7e => (*byte as char).to_string(),
                    _ => format!("\\x{byte:02x}"),
                })
                .collect();
            format!("b'{text}'")
        }
        DataType::Date32 => format!("DATE '{}'", display()),
        DataType::Timestamp(_, None) => format!("DATETIME '{}'", display().replace('T', " ")),
        DataType::Timestamp(_, Some(_)) => {
            let utc = datafusion::arrow::compute::cast(&array.slice(index, 1), &types::datetime())
                .expect("a civil time");
            let civil = array_value_to_string(&utc, 0)
                .expect("a value")
                .replace('T', " ");
            format!("TIMESTAMP '{civil}+00'")
        }
        DataType::List(_) | DataType::LargeList(_) => {
            let values = match array.data_type() {
                DataType::List(_) => array.as_list::<i32>().value(index),
                _ => array.as_list::<i64>().value(index),
            };
            let items: Vec<String> = (0..values.len()).map(|i| literal(&values, i)).collect();
            format!("[{}]", items.join(", "))
        }
        DataType::Struct(fields) => {
            let structure = array.as_struct();
            let items: Vec<String> = fields
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    format!(
                        "{} AS {}",
                        literal(structure.column(i), index),
                        field.name()
                    )
                })
                .collect();
            format!("STRUCT({})", items.join(", "))
        }
        DataType::Dictionary(_, value) => {
            let values =
                datafusion::arrow::compute::cast(&array.slice(index, 1), value).expect("values");
            literal(&values, 0)
        }
        other => panic!("the tests write no literal of {other}"),
    }
}
