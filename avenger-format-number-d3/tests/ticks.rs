//! Tick labels compared with Vega, plus tick behavior outside the fixture.

use avenger_format::{NumberFormatProvider, PreparedNumberFormatter, TickSpacing};
use avenger_format_number_d3::{D3NumberFormatProvider, D3NumberPrecision, NumberLocaleSpec};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixtures {
    locales: BTreeMap<String, NumberLocaleSpec>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    locale: String,
    spacing: String,
    spec: String,
    bits: Vec<String>,
    expected: Vec<String>,
}

fn labels(pattern: &str, values: &[f64], spacing: TickSpacing) -> Vec<String> {
    texts(
        D3NumberFormatProvider::new()
            .prepare(pattern)
            .unwrap()
            .as_ref(),
        values,
        spacing,
    )
}

fn texts(
    formatter: &dyn PreparedNumberFormatter,
    values: &[f64],
    spacing: TickSpacing,
) -> Vec<String> {
    formatter
        .format_ticks(values, spacing)
        .into_iter()
        .map(|label| label.text)
        .collect()
}

#[test]
fn matches_vega_tick_labels() {
    let fixtures: Fixtures = serde_json::from_str(include_str!("fixtures/ticks.json")).unwrap();
    let mut failures = Vec::new();
    for case in &fixtures.cases {
        let provider = D3NumberFormatProvider::new()
            .with_custom_locale(&case.locale, fixtures.locales[&case.locale].clone())
            .with_locale(&case.locale);
        let values: Vec<f64> = case
            .bits
            .iter()
            .map(|bits| f64::from_bits(u64::from_str_radix(bits, 16).unwrap()))
            .collect();
        let spacing = match case.spacing.as_str() {
            "uniform" => TickSpacing::Uniform,
            "varying" => TickSpacing::Varying,
            other => panic!("unknown spacing `{other}`"),
        };
        let actual = texts(
            provider.prepare(&case.spec).unwrap().as_ref(),
            &values,
            spacing,
        );
        if actual != case.expected {
            failures.push(format!(
                "{} {} {:?} {values:?}: {actual:?} != {:?}",
                case.locale, case.spacing, case.spec, case.expected
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn automatic_precision_applies_to_single_values_only() {
    let formatter = D3NumberFormatProvider::new()
        .with_precision(D3NumberPrecision::Automatic)
        .prepare("f")
        .unwrap();
    assert_eq!(formatter.format(0.2).text, "0.2");
    assert_eq!(
        texts(formatter.as_ref(), &[0.0, 0.2, 0.4], TickSpacing::Uniform),
        ["0.0", "0.2", "0.4"]
    );
}

#[test]
fn a_single_tick_uses_its_last_significant_digit() {
    // Vega derives the step from the domain. A degenerate domain's zero step leaves the
    // default precision, so Vega labels these ticks "0.000000" and "2.500000".
    assert_eq!(labels("f", &[0.0], TickSpacing::Uniform), ["0"]);
    assert_eq!(labels("f", &[2.5], TickSpacing::Uniform), ["2.5"]);
    assert_eq!(labels("s", &[1.2e6], TickSpacing::Uniform), ["1.2M"]);
}

#[test]
fn non_finite_ticks_leave_precision_unchanged() {
    let with = labels(
        "s",
        &[0.0, f64::NAN, 5e5, 1e6, f64::INFINITY],
        TickSpacing::Uniform,
    );
    let without = labels("s", &[0.0, 5e5, 1e6], TickSpacing::Uniform);
    assert_eq!(
        [&with[0], &with[2], &with[3]],
        [&without[0], &without[1], &without[2]]
    );
    assert_eq!(without, ["0.0M", "0.5M", "1.0M"]);
}
