use avenger_format_datetime_d3::{
    DateTimeFormatContext, DateTimeFormatOverrides, DateTimeLocaleSpec, PreparedDateTimeFormat,
    PreparedTimeMultiFormat, ResolvedDateTimeLocale,
};
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
                .unwrap()
        } else {
            PreparedDateTimeFormat::new(
                case.spec.as_str(),
                DateTimeFormatOverrides::default(),
                context,
            )
            .unwrap()
            .format_zoned(value)
            .unwrap()
        };
        assert_eq!(
            actual, case.expected,
            "{} {} {} {}",
            case.locale, case.zone, case.spec, case.value
        );
    }
}
