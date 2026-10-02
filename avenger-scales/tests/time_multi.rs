use avenger_format::NaiveDateTimeInput;
use avenger_format_datetime_d3::{D3DateTimeFormatConfig, DateTimeLocaleSpec};
use avenger_scales::formatter::{time::TimeMultiFormatSpec, DateTimeFormatAdapter};
use chrono::DateTime;
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
    spec: Option<TimeMultiFormatSpec>,
    value: i64,
    expected: String,
}

#[test]
fn matches_vega_calendar_labels() {
    let fixtures: Fixtures =
        serde_json::from_str(include_str!("fixtures/time-multi.json")).unwrap();
    for case in fixtures.cases {
        let config = D3DateTimeFormatConfig {
            locale: Some(case.locale.clone()),
            timezone: Some(case.zone.clone()),
            locales: fixtures.locales.clone(),
        };
        let formatter = DateTimeFormatAdapter::d3(config, case.spec.unwrap_or_default())
            .prepare_zoned(None)
            .unwrap();
        assert_eq!(
            formatter
                .format(DateTime::from_timestamp_millis(case.value).unwrap())
                .unwrap(),
            case.expected,
            "{} {} {}",
            case.locale,
            case.zone,
            case.value
        );
    }
}

#[test]
fn calendar_selection_matches_each_providers_fractional_precision() {
    use avenger_format_datetime_chrono::ChronoDateTimeFormatConfig;
    let d3 = DateTimeFormatAdapter::d3(D3DateTimeFormatConfig::new(), Default::default());
    let chrono =
        DateTimeFormatAdapter::chrono(ChronoDateTimeFormatConfig::new(), Default::default());
    let instant = DateTime::from_timestamp(-1, 999_500_000).unwrap();
    assert_eq!(
        d3.prepare_zoned(None).unwrap().format(instant).unwrap(),
        "1970"
    );
    assert_eq!(
        chrono.prepare_zoned(None).unwrap().format(instant).unwrap(),
        ".999500"
    );
    let civil =
        NaiveDateTimeInput::DateTime(DateTime::from_timestamp(0, 500_000).unwrap().naive_utc());
    assert_eq!(
        d3.prepare_naive(None).unwrap().format(civil).unwrap(),
        "1970"
    );
    assert_eq!(
        chrono.prepare_naive(None).unwrap().format(civil).unwrap(),
        ".000500"
    );
}

#[test]
fn civil_preparation_validates_every_calendar_branch_and_ignores_timezone() {
    let config = D3DateTimeFormatConfig::new().with_timezone("invalid/zone");
    let adapter = DateTimeFormatAdapter::d3(config.clone(), Default::default());
    assert!(adapter.prepare_naive(None).is_ok());
    assert!(adapter.prepare_zoned(None).is_err());
    let adapter = DateTimeFormatAdapter::d3(
        config,
        TimeMultiFormatSpec {
            month: Some("%Q".into()),
            ..Default::default()
        },
    );
    assert!(adapter.prepare_naive(None).is_err());
}

#[test]
fn daily_aliases_and_empty_patterns_follow_vega() {
    let date = NaiveDateTimeInput::Date(chrono::NaiveDate::from_ymd_opt(2024, 5, 6).unwrap());
    for (date_pattern, day_pattern, expected) in [
        (Some("date"), Some("day"), "date"),
        (None, Some("day"), "day"),
        (Some(""), Some("day"), "day"),
        (Some(""), Some(""), "Mon 06"),
    ] {
        let adapter = DateTimeFormatAdapter::d3(
            Default::default(),
            TimeMultiFormatSpec {
                date: date_pattern.map(str::to_owned),
                day: day_pattern.map(str::to_owned),
                ..Default::default()
            },
        );
        assert_eq!(
            adapter.prepare_naive(None).unwrap().format(date).unwrap(),
            expected
        );
    }
    let adapter = DateTimeFormatAdapter::d3(
        Default::default(),
        TimeMultiFormatSpec {
            year: Some(String::new()),
            quarter: Some("Q%q".into()),
            ..Default::default()
        },
    );
    for (month, expected) in [(1, "2024"), (4, "Q2")] {
        let date =
            NaiveDateTimeInput::Date(chrono::NaiveDate::from_ymd_opt(2024, month, 1).unwrap());
        assert_eq!(
            adapter.prepare_naive(None).unwrap().format(date).unwrap(),
            expected
        );
    }
}

#[test]
fn calendar_selection_reports_range_and_leap_second_errors() {
    use chrono::TimeZone;
    let leap = chrono::NaiveDate::MAX
        .and_hms_nano_opt(23, 59, 59, 1_500_000_000)
        .unwrap()
        .and_utc();
    for (value, zone) in [
        (DateTime::<chrono::Utc>::MAX_UTC, "Asia/Tokyo"),
        (DateTime::<chrono::Utc>::MIN_UTC, "America/New_York"),
        (leap, "UTC"),
    ] {
        let adapter = DateTimeFormatAdapter::d3(
            D3DateTimeFormatConfig::new().with_timezone(zone),
            Default::default(),
        );
        assert!(adapter.prepare_zoned(None).unwrap().format(value).is_err());
    }
    let value = chrono_tz::Asia::Tokyo
        .from_local_datetime(&chrono::NaiveDate::MIN.and_hms_opt(12, 0, 0).unwrap())
        .single()
        .unwrap()
        .with_timezone(&chrono::Utc);
    let adapter = DateTimeFormatAdapter::d3(
        D3DateTimeFormatConfig::new().with_timezone("Asia/Tokyo"),
        Default::default(),
    );
    assert!(adapter.prepare_zoned(None).unwrap().format(value).is_err());
}

#[test]
fn tick_precision_is_local_to_each_preparation() {
    use avenger_format::NumberFormatProvider;
    use avenger_format_number_d3::{D3NumberFormatConfig, D3NumberFormatProvider};
    use avenger_scales::formatter::{NumberFormatAdapter, NumberLabelContext};
    let config = D3NumberFormatConfig::new();
    let adapter = NumberFormatAdapter::d3(config.clone());
    let first = adapter
        .prepare(
            Some("f"),
            NumberLabelContext::Ticks {
                step: 0.1,
                reference_value: 1.0,
            },
        )
        .unwrap();
    let second = adapter
        .prepare(
            Some("f"),
            NumberLabelContext::Ticks {
                step: 0.01,
                reference_value: 1.0,
            },
        )
        .unwrap();
    assert_eq!(first.format(0.123).text, "0.1");
    assert_eq!(second.format(0.123).text, "0.12");
    assert_eq!(first.format(0.123).text, "0.1");
    assert_eq!(
        adapter.binding().prepare("f").unwrap().format(0.123),
        D3NumberFormatProvider
            .prepare(&config, "f")
            .unwrap()
            .format(0.123)
    );
}
