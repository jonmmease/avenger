//! Parsing against GoogleSQL: each case of `data/googlesql_parse.tsv`, which GoogleSQL's
//! `execute_query` recorded, parsed by the crate.

use avenger_format_datetime_googlesql::{
    parse_date, parse_datetime, parse_time, parse_timestamp, Error, Locale, Zone,
};

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (c, c == '\\') {
            (_, true) => match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            (c, false) => out.push(c),
        }
    }
    out
}

/// The crate's result, as the data file writes GoogleSQL's.
fn ours(function: &str, format: &str, input: &str, zone: &str) -> String {
    let locale = Locale::default();
    let result: Result<String, Error> = match function {
        "PARSE_DATE" => parse_date(format, input, &locale).map(|date| {
            let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
            (date - epoch).num_days().to_string()
        }),
        "PARSE_TIME" => {
            parse_time(format, input, &locale).map(|time| time.format("%H:%M:%S%.6f").to_string())
        }
        "PARSE_DATETIME" => parse_datetime(format, input, &locale)
            .map(|datetime| datetime.format("%Y-%m-%d %H:%M:%S%.6f").to_string()),
        _ => parse_timestamp(format, input, Zone::parse(zone).unwrap(), &locale)
            .map(|instant| instant.timestamp_micros().to_string()),
    };
    match result {
        Ok(value) => format!("ok:{value}"),
        Err(error) => format!("error:{error}"),
    }
}

#[test]
fn parsing_gives_googlesqls_values_and_errors() {
    let data = include_str!("data/googlesql_parse.tsv");
    let mut failures = Vec::new();
    let mut checked = 0;
    for line in data.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<&str> = line.split('\t').collect();
        let [function, format, input, zone, expected] = fields[..] else {
            panic!("a case of five fields: {line}");
        };
        let (format, input, expected) = (unescape(format), unescape(input), unescape(expected));
        let found = ours(function, &format, &input, zone);
        if found != expected {
            failures.push(format!(
                "{function}({format:?}, {input:?}) {zone}\n  expected {expected}\n  found    {found}"
            ));
        }
        checked += 1;
    }
    assert!(checked > 150, "{checked} cases");
    assert!(
        failures.is_empty(),
        "{} of {checked} cases differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
