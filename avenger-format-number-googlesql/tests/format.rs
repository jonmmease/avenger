//! The patterns against GoogleSQL: each case of `data/format.tsv`, which GoogleSQL's
//! `execute_query` recorded, formatted by the provider.
//!
//! A case is a pattern, a value, and the result in brackets, which keep its spaces. The float
//! conversions ran on `FLOAT64` values, and the integer conversions on `INT64` values, which the
//! provider takes as `f64`.

use avenger_format::NumberFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;

/// The number of recorded cases, so a case that the parse skips fails the test.
const CASES: usize = 2208;

/// The value of a case, such as `-0`, `1e-5`, `nan`, or `-inf`.
fn value(text: &str) -> f64 {
    match text {
        "nan" => f64::NAN,
        "inf" => f64::INFINITY,
        "-inf" => f64::NEG_INFINITY,
        _ => text.parse().unwrap_or_else(|_| panic!("a value: {text}")),
    }
}

#[test]
fn format_matches_googlesql() {
    let provider = GoogleSqlNumberFormatProvider::new();
    let data = include_str!("data/format.tsv");
    let mut count = 0;
    let mut failures = Vec::new();
    for line in data.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<&str> = line.split('\t').collect();
        let [pattern, number, result] = fields[..] else {
            panic!("a case of three fields: {line}");
        };
        let expected = (result.strip_prefix('['))
            .and_then(|result| result.strip_suffix(']'))
            .unwrap_or_else(|| panic!("a result in brackets: {line}"));
        let formatter = provider
            .prepare(pattern)
            .unwrap_or_else(|error| panic!("`{pattern}` prepares: {error}"));
        let actual = formatter.format(value(number)).text;
        if actual != expected {
            failures.push(format!("{pattern}\t{number}\t[{expected}]\t[{actual}]"));
        }
        count += 1;
    }
    assert!(
        failures.is_empty(),
        "{} of {count} cases differ (pattern, value, expected, actual):\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(count, CASES);
}
