//! `CAST ... FORMAT` against GoogleSQL: each case of `data/googlesql_cast_format.txt`, which
//! GoogleSQL's `execute_query` recorded, planned through the pass as
//! `FROM one |> SELECT CONCAT('[', CAST((<case>) AS STRING), ']') AS v`. A line holds the case,
//! a tab, and GoogleSQL's value in brackets, `NULL`, or `!` and its error.

mod pass;

use arrow::array::{Array, AsArray};

/// The value of the query of `expr`, as the data file writes it.
async fn ours(expr: &str) -> String {
    let query = format!("FROM one |> SELECT CONCAT('[', CAST(({expr}) AS STRING), ']') AS v");
    match pass::run(&query).await {
        Ok(batches) => {
            let column = batches[0].column(0);
            let column = arrow::compute::cast(column, &arrow::datatypes::DataType::Utf8).unwrap();
            let column = column.as_string::<i32>();
            match column.is_null(0) {
                true => "NULL".to_string(),
                false => column.value(0).to_string(),
            }
        }
        Err(errors) => {
            let message = &errors[0];
            let message = message.strip_prefix("Execution error: ").unwrap_or(message);
            format!("!{message}")
        }
    }
}

#[tokio::test]
async fn casts_with_formats_return_googlesqls_results() {
    let data = include_str!("data/googlesql_cast_format.txt");
    let mut failures = Vec::new();
    let mut checked = 0;
    for line in data.lines() {
        let (expr, expected) = line.split_once('\t').expect("a case, a tab, and its value");
        let found = ours(expr).await;
        if found != expected {
            failures.push(format!(
                "{expr}\n  GoogleSQL {expected}\n  avenger   {found}"
            ));
        }
        checked += 1;
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} cases differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The year and the month of today, in UTC, after Howard Hinnant's `civil_from_days`.
fn today() -> (i64, i64) {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let days = seconds.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let month = match month_from_march < 10 {
        true => month_from_march + 3,
        false => month_from_march - 9,
    };
    (year_of_era + era * 400 + i64::from(month <= 2), month)
}

/// A string without a year or a month takes today's, and one without a day the first, as
/// GoogleSQL's parsing does; `Y`, `YY` and `YYY` replace the last digits of today's year.
#[tokio::test]
async fn parsing_takes_the_missing_parts_from_today() {
    let (year, month) = today();
    let cases = [
        (
            "CAST('05' AS DATE FORMAT 'DD')",
            format!("[{year:04}-{month:02}-05]"),
        ),
        (
            "CAST('2020' AS DATE FORMAT 'YYYY')",
            format!("[2020-{month:02}-01]"),
        ),
        (
            "CAST(CAST('07' AS DATETIME FORMAT 'HH24') AS STRING FORMAT 'YYYY-MM-DD HH24:MI')",
            format!("[{year:04}-{month:02}-01 07:00]"),
        ),
        (
            "CAST('8-03-04' AS DATE FORMAT 'Y-MM-DD')",
            format!("[{:04}-03-04]", year / 10 * 10 + 8),
        ),
        (
            "CAST('18-03-04' AS DATE FORMAT 'YY-MM-DD')",
            format!("[{:04}-03-04]", year / 100 * 100 + 18),
        ),
        (
            "CAST('018-03-04' AS DATE FORMAT 'YYY-MM-DD')",
            format!("[{:04}-03-04]", year / 1000 * 1000 + 18),
        ),
    ];
    for (expr, expected) in cases {
        assert_eq!(ours(expr).await, expected, "{expr}");
    }
}

/// The number format models against GoogleSQL: each case of `data/googlesql_number_format.tsv`,
/// a type, a value, a format, and GoogleSQL's text or error, which `execute_query` recorded, of
/// `CAST(CAST(value AS type) AS STRING FORMAT format)`. avenger-sql has no `UINT64` or `FLOAT32`
/// type, and its `BIGNUMERIC`, a `Decimal256(76, 38)`, holds 38 digits before the point where
/// GoogleSQL's extremes have 39, so those cases are left out.
#[tokio::test]
async fn numbers_format_as_googlesqls() {
    let data = include_str!("data/googlesql_number_format.tsv");
    let mut failures = Vec::new();
    let mut checked = 0;
    for line in data.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        let [ty, value, format, expected] = fields[..] else {
            panic!("a line of four fields: {line}");
        };
        let integer_digits = (value.trim_matches('\'').trim_start_matches('-'))
            .split('.')
            .next()
            .map_or(0, str::len);
        if matches!(ty, "UINT64" | "FLOAT32") || (ty == "BIGNUMERIC" && integer_digits > 38) {
            continue;
        }
        let format = format.replace('\\', "\\\\").replace('\'', "\\'");
        let expr = format!("CAST(CAST({value} AS {ty}) AS STRING FORMAT '{format}')");
        let found = ours(&expr).await;
        let expected = match (
            expected.strip_prefix("ok:"),
            expected.strip_prefix("error:"),
        ) {
            (Some(text), _) => format!("[{text}]"),
            (_, Some(message)) => format!("!{message}"),
            _ => panic!("ok: or error: {expected}"),
        };
        if found != expected {
            failures.push(format!(
                "{expr}\n  GoogleSQL {expected}\n  avenger   {found}"
            ));
        }
        checked += 1;
    }
    assert!(checked > 1000, "{checked} cases");
    assert!(
        failures.is_empty(),
        "{} of {checked} cases differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
