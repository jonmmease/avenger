//! The function library against GoogleSQL: each expression of `CASES` planned through the
//! library and run, with GoogleSQL's value of it, and the binding of arguments.
//!
//! `expr` plans the expressions of these tests from the syntax tree: literals, typed literals,
//! `INTERVAL`, arrays, subscripts, `<` and `IS NOT NULL`, and calls through the library.

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
        lit, Cast, Expr, ExprFunctionExt,
    },
    prelude::SessionContext,
};

/// GoogleSQL's values of these expressions, from `execute_query --output_mode=json`, in its
/// JSON forms: an integer as a string, a date as its day number, a timestamp as microseconds,
/// and a datetime as GoogleSQL's packed bits.
const CASES: &[(&str, &str)] = &[
    ("LOG(8, 2)", "3"),
    ("LOG(100)", "4.605170185988092"),
    ("MOD(-7, 3)", "\"-1\""),
    ("DIV(-7, 2)", "\"-3\""),
    ("ROUND(2.5)", "3"),
    ("ROUND(-2.5)", "-3"),
    ("ROUND(1.2345, 2)", "1.23"),
    ("TRUNC(-2.7)", "-2"),
    ("TRUNC(1.2345, 2)", "1.23"),
    ("SIGN(-3)", "\"-1\""),
    ("POW(2, 10)", "1024"),
    ("SAFE_DIVIDE(1, 0)", "null"),
    ("GREATEST(1, NULL, 3)", "null"),
    ("LEAST(1, 2)", "\"1\""),
    ("CONCAT('a', NULL)", "null"),
    ("LENGTH('héllo')", "\"5\""),
    ("SUBSTR('abcdef', 2, 3)", "\"bcd\""),
    ("SUBSTR('abcdef', -2)", "\"ef\""),
    ("SUBSTR('abcdef', 0, 2)", "\"ab\""),
    ("SUBSTR('abcdef', -10, 3)", "\"abc\""),
    ("TRIM('xxaxx', 'x')", "\"a\""),
    ("STRPOS('abc', 'z')", "\"0\""),
    ("SPLIT('a,b,c')", "[\"a\", \"b\", \"c\"]"),
    ("REGEXP_EXTRACT('abc123', r'[0-9]+')", "\"123\""),
    ("REGEXP_EXTRACT('abc123', r'([a-z]+)')", "\"abc\""),
    ("REGEXP_EXTRACT('abc', r'z')", "null"),
    ("REGEXP_REPLACE('abc', r'(b)', r'<\\1>')", "\"a<b>c\""),
    ("IF(1 < 2, 'y', 'n')", "\"y\""),
    ("NULLIF(1, 1)", "null"),
    ("DATE_TRUNC(DATE '2024-05-17', MONTH)", "19844"),
    ("DATE_TRUNC(DATE '2024-05-17', WEEK)", "19855"),
    ("DATE_TRUNC(DATE '2024-05-17', ISOWEEK)", "19856"),
    ("DATE_TRUNC(DATE '2024-05-17', QUARTER)", "19814"),
    ("DATE_TRUNC(DATE '2024-05-17', YEAR)", "19723"),
    ("DATE_ADD(DATE '2024-01-31', INTERVAL 1 MONTH)", "19782"),
    ("DATE_SUB(DATE '2024-03-01', INTERVAL 1 DAY)", "19782"),
    ("DATE_ADD(DATE '2024-01-31', INTERVAL 2 WEEK)", "19767"),
    ("DATE_DIFF(DATE '2024-03-01', DATE '2024-01-01', DAY)", "\"60\""),
    ("DATE_DIFF(DATE '2024-03-01', DATE '2024-01-31', MONTH)", "\"2\""),
    ("DATE_DIFF(DATE '2024-01-01', DATE '2023-12-31', YEAR)", "\"1\""),
    ("DATE_DIFF(DATE '2024-01-14', DATE '2024-01-06', WEEK)", "\"2\""),
    ("TIMESTAMP_TRUNC(TIMESTAMP '2024-05-17 13:45:10+00', HOUR)", "\"1715950800000000\""),
    ("TIMESTAMP_ADD(TIMESTAMP '2024-05-17 13:45:10+00', INTERVAL 30 MINUTE)", "\"1715955310000000\""),
    ("TIMESTAMP_DIFF(TIMESTAMP '2024-05-17 13:45:10+00', TIMESTAMP '2024-05-17 12:00:00+00', MINUTE)", "\"105\""),
    ("TIMESTAMP_DIFF(TIMESTAMP '2024-05-17 13:45:10+00', TIMESTAMP '2024-05-17 12:00:00+00', HOUR)", "\"1\""),
    ("DATETIME_TRUNC(DATETIME '2024-05-17 13:45:10', DAY)", "\"142450664910356480\""),
    ("FORMAT_DATE('%Y/%m/%d', DATE '2024-05-07')", "\"2024/05/07\""),
    ("FORMAT_DATE('%b %e', DATE '2024-05-07')", "\"May  7\""),
    ("PARSE_DATE('%Y%m%d', '20240517')", "19860"),
    ("UNIX_SECONDS(TIMESTAMP '2024-01-01 00:00:00+00')", "\"1704067200\""),
    ("TIMESTAMP_SECONDS(1704067200)", "\"1704067200000000\""),
    ("DATE(2024, 5, 17)", "19860"),
    ("DATE(TIMESTAMP '2024-05-17 23:00:00+00')", "19860"),
    ("DATE(DATETIME '2024-05-17 23:00:00')", "19860"),
    ("DATETIME(2024, 5, 17, 1, 2, 3)", "\"142450669342687232\""),
    ("DATETIME(DATE '2024-05-17')", "\"142450664910356480\""),
    ("ARRAY_LENGTH([1, 2, 3])", "\"3\""),
    ("ARRAY_CONCAT([1, 2], [3])", "[\"1\", \"2\", \"3\"]"),
    ("ARRAY_REVERSE([1, 2, 3])", "[\"3\", \"2\", \"1\"]"),
    ("GENERATE_ARRAY(1, 5)", "[\"1\", \"2\", \"3\", \"4\", \"5\"]"),
    ("GENERATE_ARRAY(1, 10, 3)", "[\"1\", \"4\", \"7\", \"10\"]"),
    ("GENERATE_ARRAY(5, 1)", "[]"),
    ("[10, 20, 30][OFFSET(1)]", "\"20\""),
    ("[10, 20, 30][ORDINAL(1)]", "\"10\""),
    ("[10, 20, 30][SAFE_OFFSET(5)]", "null"),
    ("[10, 20, 30][SAFE_ORDINAL(0)]", "null"),
    ("[10, 20, 30][1]", "\"20\""),
    ("SQRT(16)", "4"),
    ("SAFE.SQRT(-1)", "null"),
    ("SAFE.LN(0)", "null"),
    ("SAFE.SUBSTR('abc', 1, -1)", "null"),
    ("SAFE.DIV(1, 0)", "null"),
    ("SAFE.MOD(1, 0)", "null"),
    ("SAFE.LOG(-1)", "null"),
    ("SAFE.POW(-8, 0.5)", "null"),
    ("SAFE.UPPER('a')", "\"A\""),
    ("FORMAT_TIMESTAMP('%Y-%m-%d %H:%M:%S', TIMESTAMP '2024-05-17 13:45:10+00', 'UTC')", "\"2024-05-17 13:45:10\""),
    ("PARSE_TIMESTAMP('%Y-%m-%d %H:%M:%S', '2024-05-17 13:45:10', 'UTC')", "\"1715953510000000\""),
    ("TIMESTAMP('2024-05-17 13:45:10', 'UTC')", "\"1715953510000000\""),
    ("TIMESTAMP(DATE '2024-05-17', 'UTC')", "\"1715904000000000\""),
    ("DATE(TIMESTAMP '2024-05-17 23:00:00+00', 'UTC')", "19860"),
    ("TIMESTAMP_TRUNC(TIMESTAMP '2024-05-17 13:45:10+00', DAY, 'UTC')", "\"1715904000000000\""),
    ("DATE_TRUNC(DATE '2024-05-17', WEEK(MONDAY))", "19856"),
    ("DATE_TRUNC(DATE '2024-05-17', WEEK(SATURDAY))", "19854"),
    ("DATE_DIFF(DATE '2024-01-14', DATE '2024-01-06', WEEK(MONDAY))", "\"1\""),
    ("DATE_DIFF(DATE '2024-01-14', DATE '2024-01-06', ISOWEEK)", "\"1\""),
    ("DATE_DIFF(DATE '2024-05-17', DATE '2024-01-01', QUARTER)", "\"1\""),
    // Time zones other than UTC, in standard and in daylight time.
    ("FORMAT_TIMESTAMP('%Y-%m-%d %H:%M:%S', TIMESTAMP '2024-05-17 13:45:10+00', 'America/Los_Angeles')", "\"2024-05-17 06:45:10\""),
    ("PARSE_TIMESTAMP('%Y-%m-%d %H:%M:%S', '2024-05-17 13:45:10', 'America/Los_Angeles')", "\"1715978710000000\""),
    ("TIMESTAMP('2024-05-17 13:45:10', 'America/Los_Angeles')", "\"1715978710000000\""),
    ("TIMESTAMP(DATE '2024-05-17', 'America/Los_Angeles')", "\"1715929200000000\""),
    ("DATE(TIMESTAMP '2024-05-18 03:00:00+00', 'America/Los_Angeles')", "19860"),
    ("TIMESTAMP_TRUNC(TIMESTAMP '2024-05-18 03:00:00+00', DAY, 'America/Los_Angeles')", "\"1715929200000000\""),
    ("DATETIME(TIMESTAMP '2024-05-17 13:45:10+00', 'America/Los_Angeles')", "\"142450693710544896\""),
    ("TIMESTAMP(DATETIME '2024-01-15 12:00:00', 'America/Los_Angeles')", "\"1705348800000000\""),
];

/// The schema of the tables of the tests, for the types of their columns.
fn schema() -> DFSchema {
    DFSchema::try_from(Schema::new(vec![
        Field::new("x", DataType::Int64, true),
        Field::new("s", DataType::Utf8, true),
    ]))
    .expect("a schema")
}

fn context(schema: &DFSchema) -> PlanContext<'_> {
    PlanContext {
        schema,
        time_zone: "UTC",
    }
}

/// The condition of `FROM t |> WHERE <text>`.
fn parse(text: &str) -> ast::Expr {
    let parse = avenger_sql_syntax::parse(&format!("FROM t |> WHERE {text}"));
    assert!(parse.ok(), "{text}: {:?}", parse.diagnostics());
    parse
        .syntax()
        .descendants()
        .find_map(ast::PipeWhere::cast)
        .and_then(|pipe| pipe.condition())
        .expect("a condition")
}

fn cast(expr: Expr, ty: DataType) -> Expr {
    Expr::Cast(Cast::new(Box::new(expr), ty))
}

/// The expression of `text`, planned through the library.
fn expr(library: &Library, text: &str) -> Expr {
    plan(library, &parse(text)).unwrap_or_else(|error| panic!("{text}: {error}"))
}

fn plan(library: &Library, expr: &ast::Expr) -> Result<Expr, CallError> {
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
            if typed.date_kw().is_some() {
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
            let lhs = plan(library, &bin.lhs().expect("an operand"))?;
            let rhs = plan(library, &bin.rhs().expect("an operand"))?;
            lhs.lt(rhs)
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
fn call_name(call: &ast::CallExpr) -> (bool, String) {
    let names = call.path().and_then(|path| path.names()).expect("a name");
    match names.as_slice() {
        [safe, name] if safe.eq_ignore_ascii_case("SAFE") => (true, name.clone()),
        [name] => (false, name.clone()),
        other => panic!("a function named {other:?}"),
    }
}

fn plan_call(library: &Library, call: &ast::CallExpr) -> Result<Expr, CallError> {
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
fn typed(expr: Expr, result: Option<DataType>) -> Expr {
    match result {
        Some(ty) => cast(expr, ty),
        None => expr,
    }
}

/// The arguments of `call`, each planned by the kind of its parameter, and named as the call
/// names it.
fn plan_args(library: &Library, call: &ast::CallExpr) -> Vec<(Option<String>, Arg)> {
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
        let kind = function.param_kind(position, named.as_deref());
        let arg = match kind {
            Some(avenger_sql::library::ParamKind::DatePart) => Arg::DatePart(date_part(&value)),
            _ => Arg::Value(plan(library, &value).expect("an argument")),
        };
        args.push((named, arg));
    }
    args
}

/// The date part that `expr` writes, as in `MONTH` or `WEEK(MONDAY)`.
fn date_part(expr: &ast::Expr) -> DatePart {
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
async fn run(expr: Expr) -> ArrayRef {
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
fn json(array: &ArrayRef, index: usize) -> String {
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
        other => panic!("the tests show no {other}"),
    }
}

fn string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The microseconds since 1970-01-01T00:00:00 of a civil time in GoogleSQL's packed form of a
/// DATETIME, whose bits hold its year, month, day, hour, minute, second and microsecond.
fn packed_datetime(micros: i64) -> i64 {
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

#[tokio::test]
async fn each_function_returns_googlesqls_value() {
    let library = Library::new();
    let mut failures = Vec::new();
    for (text, expected) in CASES {
        let planned = match plan(&library, &parse(text)) {
            Ok(planned) => planned,
            Err(error) => {
                failures.push(format!("{text}: {error}"));
                continue;
            }
        };
        let found = match tokio::spawn(run(planned)).await {
            Ok(array) => json(&array, 0),
            Err(error) => format!("a failed run: {error}"),
        };
        if &found != expected {
            failures.push(format!("{text}: GoogleSQL {expected}, avenger-sql {found}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// A table of the column `x` of integers, and the column `s` of strings.
fn table(x: &[Option<i64>], s: &[&str]) -> datafusion::arrow::array::RecordBatch {
    use datafusion::arrow::array::{Int64Array, StringArray};
    let rows = x.len().max(s.len());
    let x: Int64Array = (0..rows).map(|i| x.get(i).copied().flatten()).collect();
    let s: StringArray = (0..rows).map(|i| s.get(i).copied()).collect();
    datafusion::arrow::array::RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Int64, true),
            Field::new("s", DataType::Utf8, true),
        ])),
        vec![Arc::new(x), Arc::new(s)],
    )
    .expect("a table")
}

#[tokio::test]
async fn aggregates_return_googlesqls_values() {
    let library = Library::new();
    let numbers = [Some(1), Some(2), Some(2), Some(3), None];
    // GoogleSQL's values over UNNEST([1, 2, 2, 3, NULL]) and UNNEST(['a', 'b', 'c']).
    let cases = [
        ("SUM(x)", &numbers[..], &[][..], "\"8\""),
        ("COUNT(x)", &numbers, &[], "\"4\""),
        ("COUNT(*)", &numbers, &[], "\"5\""),
        ("AVG(x)", &numbers, &[], "2"),
        ("MIN(x)", &numbers, &[], "\"1\""),
        ("MAX(x)", &numbers, &[], "\"3\""),
        ("COUNTIF(x > 1)", &numbers, &[], "\"3\""),
        ("LOGICAL_AND(x > 0)", &numbers, &[], "true"),
        ("LOGICAL_OR(x > 2)", &numbers, &[], "true"),
        ("STRING_AGG(s)", &[], &["a", "b", "c"][..], "\"a,b,c\""),
        ("STRING_AGG(s, '-')", &[], &["a", "b", "c"], "\"a-b-c\""),
    ];
    let ctx = SessionContext::new();
    let mut failures = Vec::new();
    for (text, x, s, expected) in cases {
        // `>` reads as `<` in the evaluator, so the cases write their comparisons through it.
        let text = text
            .replace("x > 1", "1 < x")
            .replace("x > 0", "0 < x")
            .replace("x > 2", "2 < x");
        let aggregate = expr(&library, &text);
        let batches = ctx
            .read_batch(table(x, s))
            .unwrap()
            .aggregate(vec![], vec![aggregate.alias("v")])
            .unwrap()
            .collect()
            .await
            .unwrap();
        let found = json(batches[0].column(0), 0);
        if found != expected {
            failures.push(format!("{text}: GoogleSQL {expected}, avenger-sql {found}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[tokio::test]
async fn window_functions_return_googlesqls_values() {
    let library = Library::new();
    let ties = [Some(1), Some(2), Some(2), Some(3)];
    let distinct = [Some(1), Some(2), Some(3), Some(4)];
    // GoogleSQL's values over each row in the order of `x`.
    let cases = [
        ("ROW_NUMBER()", &ties, "[\"1\", \"2\", \"3\", \"4\"]"),
        ("RANK()", &ties, "[\"1\", \"2\", \"2\", \"4\"]"),
        ("DENSE_RANK()", &ties, "[\"1\", \"2\", \"2\", \"3\"]"),
        (
            "PERCENT_RANK()",
            &ties,
            "[0, 0.3333333333333333, 0.3333333333333333, 1]",
        ),
        ("CUME_DIST()", &ties, "[0.25, 0.75, 0.75, 1]"),
        ("NTILE(2)", &ties, "[\"1\", \"1\", \"2\", \"2\"]"),
        ("LAG(x)", &distinct, "[null, \"1\", \"2\", \"3\"]"),
        ("LAG(x, 2, 0)", &distinct, "[\"0\", \"0\", \"1\", \"2\"]"),
        ("LEAD(x, 1, 0)", &distinct, "[\"2\", \"3\", \"4\", \"0\"]"),
        (
            "LAG(x, default_value => 0)",
            &distinct,
            "[\"0\", \"1\", \"2\", \"3\"]",
        ),
        ("FIRST_VALUE(x)", &distinct, "[\"1\", \"1\", \"1\", \"1\"]"),
        ("LAST_VALUE(x)", &distinct, "[\"1\", \"2\", \"3\", \"4\"]"),
    ];
    let ctx = SessionContext::new();
    let mut failures = Vec::new();
    for (text, x, expected) in cases {
        // The window of the call, with its order, cast to GoogleSQL's type of its result.
        let parsed = parse(text);
        let ast::Expr::CallExpr(call) = &parsed else {
            panic!("{text} is not a call");
        };
        let (_, name) = call_name(call);
        let result = match library.get(&name).unwrap().plan(
            plan_args(&library, call),
            false,
            false,
            context(&schema()),
        ) {
            Ok(Planned::Window { func, args, result }) => (
                Expr::from(WindowFunction::new(func, args))
                    .order_by(vec![datafusion::prelude::col("x").sort(true, false)])
                    .build()
                    .unwrap(),
                result,
            ),
            other => panic!("{text}: {other:?}"),
        };
        // The cast goes in a projection after the window, as the pass places it.
        let batches = ctx
            .read_batch(table(x, &[]))
            .unwrap()
            .window(vec![result.0.alias("w")])
            .unwrap()
            .sort(vec![datafusion::prelude::col("x").sort(true, false)])
            .unwrap()
            .select(vec![
                typed(datafusion::prelude::col("w"), result.1).alias("v")
            ])
            .unwrap()
            .collect()
            .await
            .unwrap();
        let column = datafusion::arrow::compute::concat(
            &batches
                .iter()
                .map(|batch| batch.column(0).as_ref())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let values: Vec<String> = (0..column.len()).map(|i| json(&column, i)).collect();
        let found = format!("[{}]", values.join(", "));
        if found != expected {
            failures.push(format!("{text}: GoogleSQL {expected}, avenger-sql {found}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The error of planning `text`.
fn error(library: &Library, text: &str) -> CallError {
    match plan(library, &parse(text)) {
        Ok(expr) => panic!("{text} planned as {expr}"),
        Err(error) => error,
    }
}

#[test]
fn calls_bind_their_arguments_as_googlesql_does() {
    use avenger_sql::library::ErrorAt;
    let library = Library::new();
    let cases = [
        ("SUBSTR('abc', 1, 2, 3)", ErrorAt::Arg(3), "takes 3 positional arguments"),
        ("SUBSTR('abc')", ErrorAt::Call, "needs its argument `position`"),
        ("SUBSTR('abc', size => 1)", ErrorAt::Arg(1), "has no parameter named `size`"),
        ("SUBSTR(value => 'abc', 1)", ErrorAt::Arg(1), "comes before the named arguments"),
        ("SUBSTR('abc', 1, position => 2)", ErrorAt::Arg(2), "`position` has two arguments"),
        ("SAFE.PARSE_DATE('%Y', 'x')", ErrorAt::Call, "SAFE. doesn't apply to PARSE_DATE"),
        ("SAFE.SUM(x)", ErrorAt::Call, "SAFE. doesn't apply to SUM"),
        ("SUM(*)", ErrorAt::Call, "SUM takes no `*`"),
        ("REGEXP_EXTRACT('a', r'(a)(b)')", ErrorAt::Arg(1), "at most one capturing group"),
        ("REGEXP_CONTAINS('a', r'(')", ErrorAt::Arg(1), "the pattern doesn't compile"),
        ("DATE_TRUNC(DATE '2024-01-01', HOUR)", ErrorAt::Arg(1), "not HOUR"),
        ("TIMESTAMP_DIFF(TIMESTAMP '2024-01-01 00:00:00+00', TIMESTAMP '2024-01-01 00:00:00+00', MONTH)", ErrorAt::Arg(2), "not MONTH"),
        ("DIV(1.5, 2)", ErrorAt::Arg(0), "takes INT64 or NUMERIC"),
        ("bin(x, 30)", ErrorAt::Arg(1), "takes 1 positional argument"),
    ];
    for (text, at, message) in cases {
        let found = error(&library, text);
        assert_eq!(found.at, at, "{text}: {found}");
        assert!(found.message.contains(message), "{text}: {found}");
    }
}

#[test]
fn the_library_names_functions_without_regard_to_case() {
    let library = Library::new();
    let sum = library.get("sum").expect("SUM");
    assert_eq!(sum.name, "SUM");
    assert_eq!(sum.kind, avenger_sql::library::FunctionKind::Aggregate);
    let bar = library.get("BAR").expect("bar");
    assert_eq!(bar.kind, avenger_sql::library::FunctionKind::Aggregate);
    assert!(bar.doc.is_some());
    let bin = library.get("bin").expect("bin");
    assert_eq!(bin.kind, avenger_sql::library::FunctionKind::Window);
    assert!(bin.params[1..].iter().all(|param| param.named_only));
    assert_eq!(
        library.get("date_trunc").unwrap().param_kind(1, None),
        Some(avenger_sql::library::ParamKind::DatePart)
    );
    assert!(library.get("OFFSET").is_none());
}

/// The rows of `query`, run by DataFusion's SQL planner as avenger-sql does today, and of
/// `expr` as the library plans it, over the same table.
async fn same_rows(
    query: &str,
    frame: impl FnOnce(datafusion::dataframe::DataFrame) -> datafusion::dataframe::DataFrame,
) {
    let ctx = SessionContext::new();
    avenger_sql::register(&ctx).unwrap();
    let t = table(
        &[Some(3), Some(1), Some(4), Some(1), Some(5)],
        &["a", "b", "a", "c", "b"],
    );
    ctx.register_batch("t", t).unwrap();
    let expected = avenger_sql::sql(&ctx, query)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let found = frame(ctx.table("t").await.unwrap())
        .collect()
        .await
        .unwrap();
    let show = |batches: &[datafusion::arrow::array::RecordBatch]| {
        datafusion::arrow::util::pretty::pretty_format_batches(batches)
            .unwrap()
            .to_string()
    };
    assert_eq!(show(&found), show(&expected), "{query}");
}

#[tokio::test]
async fn a_mark_builds_the_chart_of_its_sql_call() {
    let library = Library::new();
    let bar = expr(&library, "bar(x => s, y => x)");
    same_rows("SELECT bar(x => s, y => x) AS chart FROM t", |frame| {
        frame.aggregate(vec![], vec![bar.alias("chart")]).unwrap()
    })
    .await;
}

#[tokio::test]
async fn bin_takes_its_options_as_named_arguments() {
    let library = Library::new();
    let bin = expr(&library, "bin(x, maxbins => 2)");
    same_rows(
        "SELECT x, bin(x, 'maxbins', 2) OVER () AS b FROM t ORDER BY x",
        |frame| {
            frame
                .window(vec![bin.alias("b")])
                .unwrap()
                .select_columns(&["x", "b"])
                .unwrap()
                .sort(vec![datafusion::prelude::col("x").sort(true, false)])
                .unwrap()
        },
    )
    .await;
}

#[test]
fn a_call_can_name_each_parameter() {
    // A parameter whose name is a reserved word, such as `default`, takes no named argument.
    let library = Library::new();
    let mut reserved = Vec::new();
    for function in library.functions() {
        for param in &function.params {
            let text = format!("FROM t |> WHERE f({} => 1)", param.name);
            if !avenger_sql_syntax::parse(&text).ok() {
                reserved.push(format!("{}({})", function.name, param.name));
            }
        }
    }
    assert!(
        reserved.is_empty(),
        "reserved parameter names: {reserved:?}"
    );
    // A parameter of avenger-chart whose name GoogleSQL reserves takes a trailing underscore,
    // as `range` of `scale` and of `scale_range` does.
    for name in ["scale", "scale_range"] {
        let function = library.get(name).expect(name);
        assert!(
            function.params.iter().any(|param| param.name == "range_"),
            "{name}"
        );
        assert!(
            function.params.iter().all(|param| param.name != "range"),
            "{name}"
        );
    }
    let ticks = library.get("ticks").expect("ticks");
    assert!(ticks.params.iter().any(|param| param.name == "at_"));
}

#[tokio::test]
async fn a_call_names_a_reserved_parameter_with_a_trailing_underscore() {
    let library = Library::new();
    let named = run(expr(
        &library,
        "scale_range(range_ => ['red', 'blue'], reverse => true)",
    ))
    .await;
    let positional = run(expr(&library, "scale_range(['red', 'blue'], true)")).await;
    assert!(!named.is_null(0));
    assert_eq!(named.as_ref(), positional.as_ref());
}
