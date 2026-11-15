//! The formats against GoogleSQL: each case of `data/googlesql_format.tsv`, which GoogleSQL's
//! `execute_query` recorded, formatted by the crate.
//!
//! The recordings come from GoogleSQL's macOS build, which passes most elements to the C library's
//! `strftime`. Where macOS's C library differs from GoogleSQL's documentation or from glibc, the
//! case expects the crate's rule: `%P` prints `am` or `pm`, an element outside the documented list
//! or with a width prints as written, and `%-Y` writes the year without zeros.

use std::collections::HashMap;

use avenger_format_datetime_googlesql::{Format, Kind};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

/// A recorded case: the function, the SQL of the value, the time zone, the format, and the result.
struct Case<'a> {
    function: &'a str,
    value: &'a str,
    zone: &'a str,
    format: String,
    expected: String,
}

fn unescape(text: &str) -> String {
    text.replace("\\n", "\n").replace("\\t", "\t")
}

fn cases() -> Vec<Case<'static>> {
    let data = include_str!("data/googlesql_format.tsv");
    (data.lines().filter(|line| !line.starts_with('#')))
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [function, value, zone, format, expected] = fields[..] else {
                panic!("a case of five fields: {line}");
            };
            Case {
                function,
                value,
                zone,
                format: unescape(format),
                expected: unescape(expected),
            }
        })
        .collect()
}

/// The text of the SQL literal `value`, such as `DATE '2024-03-05'`.
fn literal(value: &str) -> &str {
    let start = value.find('\'').expect("a quoted literal");
    &value[start + 1..value.len() - 1]
}

/// The crate's result for `case`: `ok:` and the text, or `error:` and the message.
fn ours(case: &Case) -> String {
    let kind = match case.function {
        "FORMAT_DATE" => Kind::Date,
        "FORMAT_TIME" => Kind::Time,
        "FORMAT_DATETIME" => Kind::Datetime,
        _ => Kind::Timestamp,
    };
    let format = match Format::new(&case.format, kind) {
        Ok(format) => format,
        Err(error) => return format!("error:{error}"),
    };
    let text = literal(case.value);
    let output = match kind {
        Kind::Date => format.format_date(NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()),
        Kind::Time => format.format_time(NaiveTime::parse_from_str(text, "%H:%M:%S%.f").unwrap()),
        Kind::Datetime => format
            .format_datetime(NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").unwrap()),
        Kind::Timestamp => {
            let instant =
                DateTime::parse_from_str(&format!("{text}:00"), "%Y-%m-%d %H:%M:%S%.f%:z")
                    .unwrap()
                    .with_timezone(&Utc);
            format.format_timestamp(instant, case.zone.parse().unwrap())
        }
    };
    format!("ok:{output}")
}

/// The formats whose recorded result comes from macOS's C library, and the crate's rule for
/// them: `Some` result, or `None` for the format as written.
fn linux(case: &Case, results: &HashMap<(&str, &str, &str, String), String>) -> Option<String> {
    let as_written = || Some(format!("ok:{}", case.format));
    match case.format.as_str() {
        // GoogleSQL's documentation: `am` or `pm`, the lower case of `%p`.
        "[%P]" if case.expected == "ok:[P]" => {
            let key = (case.function, case.value, case.zone, "[%p]".to_string());
            Some(results[&key].to_lowercase())
        }
        // Elements outside GoogleSQL's documented list, and widths.
        "[%v]" | "[%N]" | "[%K]" | "[%L]" | "[%f]" | "[%q]" | "[%i]" | "[%4Y]" | "[%10d]" => {
            (case.expected != format!("ok:{}", case.format))
                .then(as_written)
                .flatten()
        }
        "[%J]" if case.function == "FORMAT_TIME" => as_written(),
        // glibc's `-` writes the year without padding, where macOS's pads it to four digits.
        "[%-Y]" => Some(case.expected.replace("[0099]", "[99]")),
        _ => None,
    }
}

#[test]
fn formats_give_googlesqls_text() {
    let cases = cases();
    let results: HashMap<_, _> = (cases.iter())
        .map(|case| {
            let key = (case.function, case.value, case.zone, case.format.clone());
            (key, case.expected.clone())
        })
        .collect();
    let mut failures = Vec::new();
    for case in &cases {
        let expected = linux(case, &results).unwrap_or_else(|| case.expected.clone());
        let found = ours(case);
        if found != expected {
            failures.push(format!(
                "{}({:?}, {} {})\n  expected {expected:?}\n  found    {found:?}",
                case.function, case.format, case.value, case.zone
            ));
        }
    }
    assert!(cases.len() > 1000, "{} cases", cases.len());
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
