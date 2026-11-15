//! The provider for charts: the text of GoogleSQL's functions, and the formats that it rejects
//! where GoogleSQL's functions print an element as written.

use avenger_format::{DateTimeFormatError, DateTimeFormatProvider};
use avenger_format_datetime_googlesql::{Format, GoogleSqlDateTimeFormatProvider, Kind, Locale};
use chrono::{DateTime, NaiveDate};
use chrono_tz::America::New_York;

#[test]
fn the_provider_gives_the_text_of_googlesqls_functions() {
    let provider = GoogleSqlDateTimeFormatProvider::new().with_timezone(New_York);
    let pattern = "%a %b %e %Y %I:%M %p %Z (Q%Q, week %V)";
    let instant = DateTime::from_timestamp(1_709_622_489, 0).unwrap();
    let text = provider
        .prepare_zoned(pattern)
        .unwrap()
        .format(instant)
        .unwrap();
    let sql = Format::new(pattern, Kind::Timestamp, &Locale::default()).unwrap();
    assert_eq!(text, sql.format_timestamp(instant, New_York));
    assert_eq!(text, "Tue Mar  5 2024 02:08 AM UTC-5 (Q1, week 10)");
}

#[test]
fn the_provider_rejects_what_googlesql_prints_as_written() {
    let provider = GoogleSqlDateTimeFormatProvider::new();
    let rejected = |result: Result<(), DateTimeFormatError>| result.unwrap_err().to_string();
    let date = |pattern: &str| rejected(provider.prepare_date(pattern).map(|_| ()));
    let naive = |pattern: &str| rejected(provider.prepare_naive(pattern).map(|_| ()));
    assert_eq!(
        date("%Y %H"),
        "unsupported format specification for date input: `%H` needs a part that the value does not have"
    );
    assert_eq!(
        date("%Y %E3S"),
        "unsupported format specification for date input: `%E3S` needs a part that the value does not have"
    );
    assert_eq!(
        naive("%H:%M %Z"),
        "unsupported format specification for naive datetime input: `%Z` needs a part that the value does not have"
    );
    assert_eq!(
        naive("%Y %N"),
        "invalid datetime format specification at byte 3: `%N` is not an element of GoogleSQL's formats"
    );
    assert_eq!(
        naive("%10d"),
        "invalid datetime format specification at byte 0: `%10d` is not an element of GoogleSQL's formats"
    );
    assert_eq!(
        naive("%J"),
        "unsupported format specification for naive datetime input: GoogleSQL does not format `%J`"
    );
}

#[test]
fn the_default_calendar_patterns_prepare() {
    let provider = GoogleSqlDateTimeFormatProvider::new();
    let patterns = provider.default_calendar_patterns();
    let date = NaiveDate::from_ymd_opt(2024, 3, 5).unwrap();
    for pattern in [
        &patterns.year,
        &patterns.month,
        &patterns.week,
        &patterns.day,
    ] {
        provider
            .prepare_date(pattern)
            .unwrap()
            .format(date)
            .unwrap();
    }
    let instant = DateTime::from_timestamp_millis(1_709_622_489_123).unwrap();
    let labels: Vec<String> = [
        &patterns.year,
        &patterns.month,
        &patterns.week,
        &patterns.day,
        &patterns.hour,
        &patterns.minute,
        &patterns.second,
        &patterns.millisecond,
    ]
    .into_iter()
    .map(|pattern| {
        provider
            .prepare_zoned(pattern)
            .unwrap()
            .format(instant)
            .unwrap()
    })
    .collect();
    assert_eq!(
        labels,
        ["2024", "March", "Mar 05", "Tue 05", "07 AM", "07:08", ":09", "09.123"]
    );
}

#[test]
fn a_provider_formats_with_its_locale() {
    let provider =
        GoogleSqlDateTimeFormatProvider::new().with_locale(Locale::bundled("fr").unwrap());
    let date = NaiveDate::from_ymd_opt(2024, 3, 5).unwrap();
    let text = provider.prepare_date("%A %e %B %Y").unwrap().format(date);
    assert_eq!(text.unwrap(), "mardi  5 mars 2024");
}
