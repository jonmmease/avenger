use avenger_format::{NumberFormatError, NumberFormatProvider};
use avenger_format_number_icu::IcuNumberFormatProvider;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixtures {
    cases: Vec<Case>,
}

/// An ICU4J reference case. Rust matches ICU4J unless a compatibility exception says otherwise.
#[derive(Deserialize)]
struct Case {
    locale: String,
    skeleton: String,
    bits: String,
    /// ICU4J's label, absent when ICU4J rejects the case.
    expected: Option<String>,
    /// ICU4J's exception name.
    icu4j_error: Option<String>,
    /// Rust's error category for a case that ICU4J also rejects.
    error: Option<String>,
    /// Rust's error byte offset, when the case checks it.
    position: Option<usize>,
    compatibility_exception: Option<Exception>,
}

#[derive(Deserialize)]
struct Exception {
    expected: Option<String>,
    error: Option<String>,
    reason: String,
}

enum Outcome<'a> {
    Label(&'a str),
    Error(&'a str),
}

fn category(error: &NumberFormatError) -> (&'static str, Option<usize>) {
    match error {
        NumberFormatError::InvalidPattern { position, .. } => ("InvalidPattern", *position),
        NumberFormatError::InvalidOption { .. } => ("InvalidOption", None),
        NumberFormatError::LocaleUnavailable { .. } => ("LocaleUnavailable", None),
        NumberFormatError::InvalidLocaleData { .. } => ("InvalidLocaleData", None),
    }
}

#[test]
fn icu4j_reference() {
    let fixtures: Fixtures = serde_json::from_str(include_str!("fixtures/icu4j.json")).unwrap();
    let mut failures = Vec::new();
    for case in fixtures.cases {
        let name = format!("{} {:?}", case.locale, case.skeleton);
        let (expected, reason) = match &case.compatibility_exception {
            Some(e) => match (&e.expected, &e.error) {
                (Some(label), None) => (Outcome::Label(label), e.reason.as_str()),
                (None, Some(error)) => (Outcome::Error(error), e.reason.as_str()),
                _ => panic!("{name}: an exception records either a label or an error"),
            },
            None => match (&case.expected, &case.icu4j_error, &case.error) {
                (Some(label), None, None) => (Outcome::Label(label), "ICU4J reference"),
                (None, Some(_), Some(error)) => (Outcome::Error(error), "ICU4J also rejects it"),
                _ => panic!("{name}: record the Rust error for cases ICU4J rejects, and explain other errors with an exception"),
            },
        };
        let value = f64::from_bits(u64::from_str_radix(&case.bits, 16).unwrap());
        let actual = IcuNumberFormatProvider::new()
            .with_locale(&case.locale)
            .prepare(&case.skeleton)
            .map(|f| f.format(value).text);
        let matches = match (&actual, &expected) {
            (Ok(text), Outcome::Label(label)) => text == label,
            (Err(e), Outcome::Error(error)) => {
                let (kind, position) = category(e);
                kind == *error && case.position.is_none_or(|p| position == Some(p))
            }
            _ => false,
        };
        if !matches {
            let expected = match expected {
                Outcome::Label(label) => format!("{label:?}"),
                Outcome::Error(error) => format!("{error} at {:?}", case.position),
            };
            failures.push(format!(
                "{name} {value}: got {actual:?}, expected {expected} ({reason})"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
