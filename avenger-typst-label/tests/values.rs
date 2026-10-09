//! Values in labels: `datetime`, which builds dates and datetimes.

mod common;

use std::sync::Arc;

use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_typst_label::{
    LabelEngine, LabelError, LabelOptions, LabelParamValue, LabelParams,
};

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options())
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// A label's text, compiled with these params.
fn text(source: &str, params: LabelParams) -> String {
    let options = LabelOptions { params, ..Default::default() };
    engine().compile(source, &options).unwrap().semantic_text
}

/// The message and hints of the error that a source compiles to.
fn error(source: &str) -> (String, Vec<String>) {
    match engine().compile(source, &LabelOptions::default()).unwrap_err() {
        LabelError::Source { message, hints, .. } => (message, hints),
        other => panic!("{source}: {other:?}"),
    }
}

/// `datetime` builds the dates, naive datetimes and instants that params pass.
#[test]
fn datetime_builds_what_params_pass() {
    let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 5).unwrap();
    let datetime = date.and_hms_nano_opt(13, 4, 5, 123_456_789).unwrap();
    let time = "hour: 13, minute: 4, second: 5, nanosecond: 123456789";
    let cases = [
        (
            "datetime(year: 2024, month: 1, day: 5)".to_string(),
            LabelParamValue::Date(date),
            "%B %-d, %Y",
        ),
        (
            format!("datetime(year: 2024, month: 1, day: 5, {time})"),
            LabelParamValue::NaiveDateTime(datetime),
            "%Y-%m-%d %H:%M:%S.%L",
        ),
        (
            format!("datetime(year: 2024, month: 1, day: 5, {time}, utc: true)"),
            LabelParamValue::ZonedDateTime(datetime.and_utc()),
            "%Y-%m-%dT%H:%M:%S.%L %Q",
        ),
    ];
    for (built, value, pattern) in cases {
        let params = LabelParams::from([("value".to_string(), value)]);
        assert_eq!(
            text(&format!("#datetimefmt({built}, \"{pattern}\")"), LabelParams::new()),
            text(&format!("#datetimefmt(value, \"{pattern}\")"), params),
            "{built}"
        );
    }
}

/// `datetime`'s errors are upstream's, but for the arguments a label adds and for times without
/// dates, which labels don't have.
#[test]
fn datetime_reports_incomplete_and_invalid_arguments() {
    let date = "year: 2024, month: 1, day: 5";
    let add_time = "add the `hour`, `minute`, and `second` arguments to get a valid time";
    let add_date = "add the `year`, `month`, and `day` arguments to get a valid date";
    let cases = [
        (
            "#datetime(year: 2024)".to_string(),
            "date is incomplete",
            vec!["add the `month` and `day` arguments to get a valid date"],
        ),
        (
            format!("#datetime({date}, minute: 3)"),
            "time is incomplete",
            vec!["add the `hour` and `second` arguments to get a valid time"],
        ),
        (
            "#datetime(year: 2023, month: 2, day: 29)".to_string(),
            "date is invalid",
            vec![],
        ),
        (
            format!("#datetime({date}, hour: 24, minute: 0, second: 0)"),
            "time is invalid",
            vec![],
        ),
        (
            "#datetime(hour: 1, minute: 2, second: 3)".to_string(),
            "times without dates are not supported in labels",
            vec![add_date],
        ),
        (
            format!("#datetime({date}, nanosecond: 1)"),
            "`nanosecond` needs a time",
            vec![add_time],
        ),
        (format!("#datetime({date}, utc: true)"), "`utc` needs a time", vec![add_time]),
        (
            "#datetime()".to_string(),
            "at least one of date or time must be fully specified",
            vec![add_time, add_date],
        ),
    ];
    for (source, message, hints) in cases {
        assert_eq!(
            error(&source),
            (message.to_string(), hints.iter().map(|h| h.to_string()).collect()),
            "{source}"
        );
    }
}
