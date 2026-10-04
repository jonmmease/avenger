use avenger_format::{NumberFormatError, NumberFormatProvider};
use avenger_format_number_icu::IcuNumberFormatProvider;
use serde::Deserialize;
use std::collections::BTreeMap;

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

/// ICU4J's labels for every skeleton and locale at each input.
#[derive(Deserialize)]
struct Grid {
    bits: Vec<String>,
    labels: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

/// Avenger's label for each grid input where it differs from ICU4J, by skeleton and locale.
type Differences = BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>;

/// The grid matches ICU4J except where the recorded differences say otherwise. Each run writes
/// the current differences to `tests/output/`; copying that file over the fixture accepts them.
#[test]
fn icu4j_grid() {
    let grid: Grid = serde_json::from_str(include_str!("fixtures/icu4j_grid.json")).unwrap();
    let recorded: Differences =
        serde_json::from_str(include_str!("fixtures/icu4j_grid_differences.json")).unwrap();
    let mut actual = Differences::new();
    let mut changes = Vec::new();
    for (skeleton, locales) in &grid.labels {
        for (locale, labels) in locales {
            let formatter = IcuNumberFormatProvider::new()
                .with_locale(locale)
                .prepare(skeleton)
                .unwrap_or_else(|e| panic!("{locale} {skeleton:?}: {e}"));
            for (bits, icu4j) in grid.bits.iter().zip(labels) {
                let value = f64::from_bits(u64::from_str_radix(bits, 16).unwrap());
                let label = formatter.format(value).text;
                let expected = recorded
                    .get(skeleton)
                    .and_then(|l| l.get(locale))
                    .and_then(|b| b.get(bits))
                    .unwrap_or(icu4j);
                if &label != expected {
                    changes.push(format!(
                        "{locale} {skeleton:?} {value}: got {label:?}, expected {expected:?} (ICU4J {icu4j:?})"
                    ));
                }
                if &label != icu4j {
                    let row = actual.entry(skeleton.clone()).or_default();
                    row.entry(locale.clone())
                        .or_default()
                        .insert(bits.clone(), label);
                }
            }
        }
    }
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    std::fs::create_dir_all(format!("{dir}/output")).unwrap();
    let output = format!("{dir}/output/icu4j_grid_differences.json");
    std::fs::write(
        &output,
        serde_json::to_string_pretty(&actual).unwrap() + "\n",
    )
    .unwrap();
    assert!(
        actual == recorded,
        "{} grid rows changed:\n{}\nTo accept them: cp {output} {dir}/fixtures/",
        changes.len(),
        changes.join("\n")
    );
}
