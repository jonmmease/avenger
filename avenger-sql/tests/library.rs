//! The function library against GoogleSQL: each expression of `CASES`, and aggregate and window
//! calls, planned through the pass and run, with GoogleSQL's values, and the binding of
//! arguments.

mod pass;

use std::sync::Arc;

use avenger_sql::{catalog::MemoryCatalog, library::Library};
use datafusion::{
    arrow::{
        array::{Int64Array, RecordBatch, StringArray},
        datatypes::{DataType, Field, Schema},
    },
    datasource::MemTable,
};

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
    // A format reads the type of its value, whatever the function.
    ("FORMAT_DATE('%Y %H %Z', DATETIME '2024-05-07 13:00:00')", "\"2024 13 %Z\""),
    ("FORMAT_DATETIME('%Y %H %Z', TIMESTAMP '2024-05-07 23:00:00+00', 'Asia/Kolkata')", "\"2024 04 UTC+0530\""),
    ("FORMAT_TIMESTAMP('%c', DATETIME '2024-05-07 13:00:00')", "\"Tue May  7 13:00:00 2024\""),
    ("FORMAT_DATE('%P %H', DATE '2024-05-07')", "\"%P %H\""),
    // A time zone is a name in any case or an offset.
    ("FORMAT_TIMESTAMP('%H:%M %Z', TIMESTAMP '2024-01-01 00:00:00+00', '+05:30')", "\"05:30 UTC+0530\""),
    ("FORMAT_TIMESTAMP('%H:%M %Z', TIMESTAMP '2024-01-01 00:00:00+00', 'america/new_york')", "\"19:00 UTC-5\""),
    ("PARSE_TIMESTAMP('%Y-%m-%d %H:%M', '2024-03-10 02:30', 'America/New_York')", "\"1710055800000000\""),
    ("PARSE_DATETIME('%E4Y-%m-%dT%H:%M:%E*S', '2024-05-17T13:45:10.25')", "\"142450723775565968\""),
    ("PARSE_TIME('%H:%M:%E*S', '13:04:05.123456')", "\"56108376640\""),
    ("SAFE.FORMAT_DATE('%J', DATE '2024-05-07')", "null"),
    ("SAFE.PARSE_DATE('%Y', 'x')", "null"),
    ("FORMAT_DATE(NULL, DATE '2024-05-07')", "null"),
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
    let mut failures = Vec::new();
    for (text, expected) in CASES {
        let found = match pass::value(text).await {
            Ok(array) => pass::values::json(&array, 0),
            Err(errors) => format!("{errors:?}"),
        };
        if &found != expected {
            failures.push(format!("{text}: GoogleSQL {expected}, avenger-sql {found}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The catalog of the pass's tests with the table `cases` of the column `x` of integers and the
/// column `s` of strings.
fn table(x: &[Option<i64>], s: &[&str]) -> MemoryCatalog {
    let rows = x.len().max(s.len());
    let x: Int64Array = (0..rows).map(|i| x.get(i).copied().flatten()).collect();
    let s: StringArray = (0..rows).map(|i| s.get(i).copied()).collect();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Int64, true),
            Field::new("s", DataType::Utf8, true),
        ])),
        vec![Arc::new(x), Arc::new(s)],
    )
    .expect("a table");
    let table = MemTable::try_new(batch.schema(), vec![vec![batch]]).unwrap();
    pass::catalog().with_table(&["cases"], Arc::new(table))
}

/// The values of the first column of `query` over `catalog`, in their order, in GoogleSQL's
/// JSON form.
async fn values(catalog: &MemoryCatalog, query: &str) -> Vec<String> {
    let batches = (pass::run_in(&pass::session(), catalog, query).await)
        .unwrap_or_else(|errors| panic!("{query}: {errors:?}"));
    (batches.iter())
        .flat_map(|batch| (0..batch.num_rows()).map(|i| pass::values::json(batch.column(0), i)))
        .collect()
}

#[tokio::test]
async fn aggregates_return_googlesqls_values() {
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
    let mut failures = Vec::new();
    for (text, x, s, expected) in cases {
        let catalog = table(x, s);
        let found = values(&catalog, &format!("FROM cases |> AGGREGATE {text} AS v")).await;
        if found != [expected] {
            failures.push(format!(
                "{text}: GoogleSQL {expected}, avenger-sql {found:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[tokio::test]
async fn window_functions_return_googlesqls_values() {
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
    let mut failures = Vec::new();
    for (text, x, expected) in cases {
        let catalog = table(x, &[]);
        // The rows of two equal `x` are in the order of the window's values.
        let query = format!(
            "FROM cases |> EXTEND {text} OVER (ORDER BY x) AS v |> ORDER BY x, v |> SELECT v"
        );
        let found = format!("[{}]", values(&catalog, &query).await.join(", "));
        if found != expected {
            failures.push(format!("{text}: GoogleSQL {expected}, avenger-sql {found}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn calls_bind_their_arguments_as_googlesql_does() {
    // Each call, as `FROM t |> SELECT <call>`, the text of its error, at an argument or at the
    // function's name, and its message.
    let cases = [
        ("SUBSTR('abc', 1, 2, 3)", "3", "takes 3 positional arguments"),
        ("SUBSTR('abc')", "SUBSTR", "needs its argument `position`"),
        ("SUBSTR('abc', size => 1)", "size => 1", "has no parameter named `size`"),
        ("SUBSTR(value => 'abc', 1)", "1", "comes before the named arguments"),
        ("SUBSTR('abc', 1, position => 2)", "position => 2", "`position` has two arguments"),
        ("SAFE.SUM(x)", "SUM", "SAFE. doesn't apply to SUM"),
        ("SUM(*)", "SUM", "SUM takes no `*`"),
        ("REGEXP_EXTRACT('a', r'(a)(b)')", "r'(a)(b)'", "at most one capturing group"),
        ("REGEXP_CONTAINS('a', r'(')", "r'('", "the pattern doesn't compile"),
        ("DATE_TRUNC(DATE '2024-01-01', HOUR)", "HOUR", "not HOUR"),
        ("TIMESTAMP_DIFF(TIMESTAMP '2024-01-01 00:00:00+00', TIMESTAMP '2024-01-01 00:00:00+00', MONTH)", "MONTH", "not MONTH"),
        ("DIV(1.5, 2)", "DIV", "no form of DIV takes (FLOAT64, INT64)"),
        ("bin(x, 30) OVER ()", "30", "takes 1 positional argument"),
    ];
    for (text, at, message) in cases {
        let errors = pass::errors(&format!("FROM t |> SELECT {text}"));
        let (found, found_at) = errors.first().unwrap_or_else(|| panic!("{text} planned"));
        assert_eq!(found_at, at, "{text}: {found}");
        assert!(found.contains(message), "{text}: {found}");
    }
}

#[tokio::test]
async fn formats_and_parses_fail_with_googlesqls_errors() {
    // GoogleSQL's errors of these calls, from `execute_query`.
    let cases = [
        (
            "FORMAT_DATE('%J', DATE '2024-05-07')",
            "Format element %J not supported yet",
        ),
        (
            "PARSE_DATE('%Y', 'x')",
            "Failed to parse input string \"x\"",
        ),
        (
            "PARSE_TIME('%H:%M:%E3S', '13:04:05.123456')",
            "Failed to parse input string \"13:04:05.123456\"",
        ),
        (
            "FORMAT_TIMESTAMP('%H', TIMESTAMP '2024-01-01 00:00:00+00', 'xyz')",
            "Invalid time zone: xyz",
        ),
    ];
    for (text, message) in cases {
        let errors = pass::value(text).await.expect_err(text);
        assert!(
            errors.iter().any(|error| error.contains(message)),
            "{text}: {errors:?}"
        );
    }
}

#[tokio::test]
async fn a_format_from_a_column_prepares_each_row() {
    let catalog = table(&[], &["%Y", "%m", "%Q"]);
    let query = "FROM cases |> SELECT FORMAT_DATE(s, DATE '2024-05-07') AS v";
    assert_eq!(
        values(&catalog, query).await,
        ["\"2024\"", "\"05\"", "\"2\""]
    );
}

#[test]
fn a_cast_with_a_format_is_unsupported() {
    let found = pass::errors("FROM t |> SELECT CAST(x AS STRING FORMAT '999') AS v");
    let message =
        "avenger-sql doesn't take CAST ... FORMAT: format and parse dates and times with \
                   FORMAT_DATE, FORMAT_TIME, FORMAT_DATETIME, FORMAT_TIMESTAMP, PARSE_DATE, \
                   PARSE_TIME, PARSE_DATETIME and PARSE_TIMESTAMP, and format the numbers and dates of a chart in its guides";
    assert_eq!(found, [(message.to_string(), "FORMAT '999'".to_string())]);
}

#[test]
fn the_library_names_functions_without_regard_to_case() {
    let library = Library::new();
    assert_eq!(library.get("sum").expect("SUM").name, "SUM");
    assert_eq!(library.get("BAR").expect("bar").name, "bar");
    assert!(library.get("OFFSET").is_none());
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
