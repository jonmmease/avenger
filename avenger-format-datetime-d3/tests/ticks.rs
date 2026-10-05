//! Time axis labels compared with Vega's calendar multi-format.

use avenger_format::{CalendarPatterns, DateTimeFormatProvider};
use avenger_format_datetime_d3::{D3DateTimeFormatProvider, DateTimeLocaleSpec};
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixtures {
    locales: BTreeMap<String, DateTimeLocaleSpec>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    locale: String,
    zone: String,
    /// Vega's unit-keyed overrides, or `None` for its defaults.
    spec: Option<BTreeMap<String, String>>,
    values: Vec<i64>,
    expected: Vec<String>,
}

/// Apply Vega's overrides with the builders. Vega's `date` takes precedence over `day`, and
/// quarter starts are ordinary months, so a `quarter` override must match `month`.
fn with_overrides(defaults: CalendarPatterns, spec: &BTreeMap<String, String>) -> CalendarPatterns {
    spec.iter()
        .fold(defaults, |patterns, (unit, pattern)| match unit.as_str() {
            "milliseconds" => patterns.with_millisecond(pattern),
            "seconds" => patterns.with_second(pattern),
            "minutes" => patterns.with_minute(pattern),
            "hours" => patterns.with_hour(pattern),
            "day" if spec.contains_key("date") => patterns,
            "date" | "day" => patterns.with_day(pattern),
            "week" => patterns.with_week(pattern),
            "month" => patterns.with_month(pattern),
            "quarter" => {
                assert_eq!(spec.get("month"), Some(pattern), "quarter override");
                patterns
            }
            "year" => patterns.with_year(pattern),
            other => panic!("unsupported Vega override `{other}`"),
        })
}

#[test]
fn matches_vega_time_axis_labels() {
    let fixtures: Fixtures = serde_json::from_str(include_str!("fixtures/ticks.json")).unwrap();
    let mut failures = Vec::new();
    for case in &fixtures.cases {
        let provider = D3DateTimeFormatProvider::new()
            .with_custom_locale(&case.locale, fixtures.locales[&case.locale].clone())
            .with_locale(&case.locale)
            .with_timezone(case.zone.parse::<Tz>().unwrap());
        let defaults = provider.default_calendar_patterns();
        let patterns = match &case.spec {
            Some(spec) => with_overrides(defaults, spec),
            None => defaults,
        };
        let format = patterns.prepare_zoned(&provider).unwrap();
        let actual: Vec<String> = case
            .values
            .iter()
            .map(|&millis| {
                format
                    .format(DateTime::from_timestamp_millis(millis).unwrap())
                    .unwrap()
            })
            .collect();
        if actual != case.expected {
            failures.push(format!(
                "{} {} {:?}: {actual:?} != {:?}",
                case.zone, case.locale, case.spec, case.expected
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
fn dates_and_naive_datetimes_use_the_same_patterns() {
    let provider = D3DateTimeFormatProvider::new();
    let patterns = provider.default_calendar_patterns();
    let dates = patterns.prepare_date(&provider).unwrap();
    let naive = patterns.prepare_naive(&provider).unwrap();
    for (date, expected) in [
        ((2024, 1, 1), "2024"),
        ((2024, 2, 1), "February"),
        ((2024, 3, 3), "Mar 03"),
        ((2024, 3, 5), "Tue 05"),
    ] {
        let date = NaiveDate::from_ymd_opt(date.0, date.1, date.2).unwrap();
        assert_eq!(dates.format(date).unwrap(), expected);
        assert_eq!(
            naive.format(date.and_hms_opt(0, 0, 0).unwrap()).unwrap(),
            expected
        );
    }
    let afternoon = NaiveDate::from_ymd_opt(2024, 3, 5)
        .unwrap()
        .and_hms_milli_opt(15, 15, 30, 250)
        .unwrap();
    assert_eq!(naive.format(afternoon).unwrap(), ".250");
}
