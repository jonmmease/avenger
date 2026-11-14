//! The function library against GoogleSQL: each expression of `CASES` planned through the
//! library and run, with GoogleSQL's value of it, and the binding of arguments.

mod eval;

use std::sync::Arc;

use avenger_sql::library::{CallError, Library, Planned};
use avenger_sql_syntax::ast;
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    logical_expr::{expr::WindowFunction, Expr, ExprFunctionExt},
    prelude::SessionContext,
};
use eval::*;

/// GoogleSQL's values of these expressions, from `execute_query --output_mode=json`, in its
/// JSON forms: an integer as a string, a date as its day number, a timestamp as microseconds,
/// and a datetime as GoogleSQL's packed bits.
const CASES: &[(&str, &str)] = &[
    ("GREATEST(1, NULL, 3)", "null"),
    ("SUBSTR('abcdef', 2, 3)", "\"bcd\""),
    ("SUBSTR('abcdef', -2)", "\"ef\""),
    ("SUBSTR('abcdef', 0, 2)", "\"ab\""),
    ("SUBSTR('abcdef', -10, 3)", "\"abc\""),
    ("REGEXP_EXTRACT('abc123', r'([a-z]+)')", "\"abc\""),
    ("REGEXP_EXTRACT('abc', r'z')", "null"),
    ("REGEXP_REPLACE('abc', r'(b)', r'<\\1>')", "\"a<b>c\""),
    ("DATE_TRUNC(DATE '2024-05-17', ISOWEEK)", "19856"),
    ("DATE_DIFF(DATE '2024-01-14', DATE '2024-01-06', WEEK)", "\"2\""),
    ("DATETIME_TRUNC(DATETIME '2024-05-17 13:45:10', DAY)", "\"142450664910356480\""),
    ("DATE(TIMESTAMP '2024-05-17 23:00:00+00')", "19860"),
    ("DATETIME(2024, 5, 17, 1, 2, 3)", "\"142450669342687232\""),
    ("[10, 20, 30][OFFSET(1)]", "\"20\""),
    ("[10, 20, 30][ORDINAL(1)]", "\"10\""),
    ("[10, 20, 30][SAFE_OFFSET(5)]", "null"),
    ("[10, 20, 30][SAFE_ORDINAL(0)]", "null"),
    ("SAFE.LN(0)", "null"),
    ("SAFE.SUBSTR('abc', 1, -1)", "null"),
    ("SAFE.DIV(1, 0)", "null"),
    ("SAFE.LOG(-1)", "null"),
    ("SAFE.POW(-8, 0.5)", "null"),
    ("SAFE.UPPER('a')", "\"A\""),
    ("DATE_TRUNC(DATE '2024-05-17', WEEK(SATURDAY))", "19854"),
    ("DATE_DIFF(DATE '2024-01-14', DATE '2024-01-06', WEEK(MONDAY))", "\"1\""),
    ("DATE_DIFF(DATE '2024-01-14', DATE '2024-01-06', ISOWEEK)", "\"1\""),
    ("DATE_DIFF(DATE '2024-05-17', DATE '2024-01-01', QUARTER)", "\"1\""),
    // Time zones other than UTC, in standard and in daylight time.
    ("FORMAT_TIMESTAMP('%Y-%m-%d %H:%M:%S', TIMESTAMP '2024-05-17 13:45:10+00', 'America/Los_Angeles')", "\"2024-05-17 06:45:10\""),
    ("TIMESTAMP('2024-05-17 13:45:10', 'America/Los_Angeles')", "\"1715978710000000\""),
    ("DATETIME(TIMESTAMP '2024-05-17 13:45:10+00', 'America/Los_Angeles')", "\"142450693710544896\""),
    ("TIMESTAMP(DATETIME '2024-01-15 12:00:00', 'America/Los_Angeles')", "\"1705348800000000\""),
];

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
        ("COUNTIF(x > 1)", &numbers, &[], "\"3\""),
        ("STRING_AGG(s)", &[], &["a", "b", "c"][..], "\"a,b,c\""),
        ("STRING_AGG(s, '-')", &[], &["a", "b", "c"], "\"a-b-c\""),
    ];
    let ctx = SessionContext::new();
    let mut failures = Vec::new();
    for (text, x, s, expected) in cases {
        let aggregate = expr(&library, text);
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
        ("NTILE(2)", &ties, "[\"1\", \"1\", \"2\", \"2\"]"),
        ("LAG(x)", &distinct, "[null, \"1\", \"2\", \"3\"]"),
        ("LAG(x, 2, 0)", &distinct, "[\"0\", \"0\", \"1\", \"2\"]"),
        (
            "LAG(x, default_value => 0)",
            &distinct,
            "[\"0\", \"1\", \"2\", \"3\"]",
        ),
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
        ("SAFE.SUM(x)", ErrorAt::Call, "SAFE. doesn't apply to SUM"),
        ("SUM(*)", ErrorAt::Call, "SUM takes no `*`"),
        ("REGEXP_EXTRACT('a', r'(a)(b)')", ErrorAt::Arg(1), "at most one capturing group"),
        ("REGEXP_CONTAINS('a', r'(')", ErrorAt::Arg(1), "the pattern doesn't compile"),
        ("DATE_TRUNC(DATE '2024-01-01', HOUR)", ErrorAt::Arg(1), "not HOUR"),
        ("TIMESTAMP_DIFF(TIMESTAMP '2024-01-01 00:00:00+00', TIMESTAMP '2024-01-01 00:00:00+00', MONTH)", ErrorAt::Arg(2), "not MONTH"),
        ("DIV(1.5, 2)", ErrorAt::Call, "no form of DIV takes (FLOAT64, INT64)"),
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
    assert_eq!(library.get("sum").expect("SUM").name, "SUM");
    assert_eq!(library.get("BAR").expect("bar").name, "bar");
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
fn a_call_can_name_each_function_and_parameter() {
    // A function or a parameter whose name is a reserved word, such as `select` or `default`,
    // takes a trailing underscore, since a call can't name it otherwise.
    let library = Library::new();
    let mut reserved = Vec::new();
    for function in library.functions() {
        if !avenger_sql_syntax::parse(&format!("FROM t |> WHERE {}(1)", function.name)).ok() {
            reserved.push(function.name.clone());
        }
        for param in &function.params {
            let text = format!("FROM t |> WHERE f({} => 1)", param.name);
            if !avenger_sql_syntax::parse(&text).ok() {
                reserved.push(format!("{}({})", function.name, param.name));
            }
        }
    }
    assert!(reserved.is_empty(), "reserved names: {reserved:?}");
    assert!(library.get("select_").is_some());
}
