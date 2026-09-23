use avenger_format_datetime_d3::{
    DateTimeFormatContext, DateTimeFormatOverrides, DateTimeLocaleSpec, NaiveDateTimeInput,
    PreparedDateTimeFormat, PreparedTimeMultiFormat, ResolvedDateTimeLocale,
};
use chrono::{DateTime, NaiveDate};
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
    mode: String,
    spec: serde_json::Value,
    value: i64,
    expected: String,
}

#[test]
fn matches_d3_time_format() {
    let fixtures: Fixtures = serde_json::from_str(include_str!("fixtures/upstream.json")).unwrap();
    let locales: BTreeMap<_, _> = fixtures
        .locales
        .into_iter()
        .map(|(id, spec)| (id.clone(), ResolvedDateTimeLocale::new(id, spec).unwrap()))
        .collect();
    for case in fixtures.cases {
        let context =
            DateTimeFormatContext::new(&locales[&case.locale], case.zone.parse().unwrap());
        let value = DateTime::from_timestamp_millis(case.value).unwrap();
        let actual = if case.mode == "multi" {
            let spec = if case.spec.is_null() {
                Default::default()
            } else {
                serde_json::from_value(case.spec.clone()).unwrap()
            };
            PreparedTimeMultiFormat::new(&spec, context)
                .unwrap()
                .format_zoned(value)
                .text
        } else {
            PreparedDateTimeFormat::new(
                case.spec.as_str(),
                DateTimeFormatOverrides::default(),
                context,
            )
            .unwrap()
            .format_zoned(value)
            .text
        };
        assert_eq!(
            actual, case.expected,
            "{} {} {} {}",
            case.locale, case.zone, case.spec, case.value
        );
    }
}

#[test]
fn locale_expansion_and_civil_input_requirements() {
    let recursive = DateTimeLocaleSpec {
        date: "%c".into(),
        date_time: "%x".into(),
        ..Default::default()
    };
    assert!(ResolvedDateTimeLocale::new("cycle", recursive).is_err());
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, chrono_tz::America::New_York);
    let date = NaiveDateTimeInput::Date(NaiveDate::from_ymd_opt(2024, 3, 10).unwrap());
    for spec in ["%Z", "%Q", "%s"] {
        let format = PreparedDateTimeFormat::new(Some(spec), Default::default(), context).unwrap();
        assert!(format.validate_naive().is_err());
        assert!(format.format_naive(date).is_err());
    }
    let format =
        PreparedDateTimeFormat::new(Some("%Y-%m-%d"), Default::default(), context).unwrap();
    assert_eq!(format.format_naive(date).unwrap().text, "2024-03-10");
}

#[test]
fn submillisecond_instants_use_javascript_date_precision() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, chrono_tz::UTC);
    let format =
        PreparedDateTimeFormat::new(Some("%Q %s %L %f"), Default::default(), context).unwrap();
    let just_before_epoch = DateTime::from_timestamp(-1, 999_500_000).unwrap();
    assert_eq!(
        format.format_zoned(just_before_epoch).text,
        "0 0 000 000000"
    );
    let multi = PreparedTimeMultiFormat::new(&Default::default(), context).unwrap();
    assert_eq!(multi.format_zoned(just_before_epoch).text, "1970");
}
