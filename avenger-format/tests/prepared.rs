//! Labeling values through the enum of prepared formatters.

use avenger_format::{
    DateTimeFormatError, DateTimeInputKind, FormatError, FormatValues, FormattedNumber,
    PreparedDateFormatter, PreparedFormatter, PreparedNaiveDateTimeFormatter,
    PreparedNumberFormatter, PreparedZonedDateTimeFormatter, TickSpacing, ValueKind,
};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Utc};
use std::sync::Arc;

/// Labels ticks with their spacing, so tests can see which method ran.
#[derive(Debug)]
struct Spacing;

impl PreparedNumberFormatter for Spacing {
    fn format(&self, value: f64) -> FormattedNumber {
        FormattedNumber::plain(format!("{value}"))
    }

    fn format_ticks(&self, values: &[f64], spacing: TickSpacing) -> Vec<FormattedNumber> {
        values
            .iter()
            .map(|value| FormattedNumber::plain(format!("{spacing:?} {value}")))
            .collect()
    }
}

/// Formats with a chrono pattern and rejects years before 1900.
#[derive(Debug)]
struct Chrono(&'static str);

impl PreparedDateFormatter for Chrono {
    fn format(&self, value: NaiveDate) -> Result<String, DateTimeFormatError> {
        if value.year() < 1900 {
            return Err(DateTimeFormatError::UnsupportedValue {
                input: DateTimeInputKind::Date,
                message: "too early".into(),
            });
        }
        Ok(value.format(self.0).to_string())
    }
}

impl PreparedNaiveDateTimeFormatter for Chrono {
    fn format(&self, value: NaiveDateTime) -> Result<String, DateTimeFormatError> {
        Ok(value.format(self.0).to_string())
    }
}

impl PreparedZonedDateTimeFormatter for Chrono {
    fn format(&self, value: DateTime<Utc>) -> Result<String, DateTimeFormatError> {
        Ok(value.format(self.0).to_string())
    }

    fn timezone(&self) -> chrono_tz::Tz {
        chrono_tz::UTC
    }
}

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

#[test]
fn numbers_use_the_formatters_tick_method() {
    let format = PreparedFormatter::from(Arc::new(Spacing) as Arc<dyn PreparedNumberFormatter>);
    assert_eq!(format.kind(), ValueKind::Number);
    for (spacing, expected) in [
        (TickSpacing::Uniform, ["Uniform 0", "Uniform 0.5"]),
        (TickSpacing::Varying, ["Varying 0", "Varying 0.5"]),
    ] {
        assert_eq!(
            format.format_ticks(FormatValues::Numbers(&[0.0, 0.5]), spacing),
            Ok(expected.map(String::from).to_vec())
        );
    }
}

#[test]
fn datetimes_format_each_value_and_leave_missing_values_empty() {
    let day = date(2024, 3, 5);
    let noon = day.and_hms_opt(12, 0, 0).unwrap();
    let cases = [
        (
            PreparedFormatter::from(Arc::new(Chrono("%b %d")) as Arc<dyn PreparedDateFormatter>),
            FormatValues::Dates(&[Some(day), None]),
            ValueKind::Date,
            ["Mar 05", ""],
        ),
        (
            PreparedFormatter::from(
                Arc::new(Chrono("%H:%M")) as Arc<dyn PreparedNaiveDateTimeFormatter>
            ),
            FormatValues::NaiveDateTimes(&[Some(noon), None]),
            ValueKind::NaiveDateTime,
            ["12:00", ""],
        ),
        (
            PreparedFormatter::from(
                Arc::new(Chrono("%H:%M")) as Arc<dyn PreparedZonedDateTimeFormatter>
            ),
            FormatValues::ZonedDateTimes(&[Some(noon.and_utc()), None]),
            ValueKind::ZonedDateTime,
            ["12:00", ""],
        ),
    ];
    for (format, values, kind, expected) in cases {
        assert_eq!(format.kind(), kind);
        assert_eq!(values.kind(), kind);
        for spacing in [TickSpacing::Uniform, TickSpacing::Varying] {
            assert_eq!(
                format.format_ticks(values, spacing),
                Ok(expected.map(String::from).to_vec())
            );
        }
    }
}

#[test]
fn values_of_another_kind_are_rejected() {
    let number = PreparedFormatter::Number(Arc::new(Spacing));
    let zoned = PreparedFormatter::ZonedDateTime(Arc::new(Chrono("%H")));
    let error = number
        .format_ticks(
            FormatValues::Dates(&[Some(date(2024, 1, 1))]),
            TickSpacing::Uniform,
        )
        .unwrap_err();
    assert_eq!(
        error,
        FormatError::Mismatch {
            formatter: ValueKind::Number,
            values: ValueKind::Date
        }
    );
    assert_eq!(
        error.to_string(),
        "a number formatter cannot label date values"
    );
    assert_eq!(
        zoned.format_ticks(FormatValues::Numbers(&[1.0]), TickSpacing::Varying),
        Err(FormatError::Mismatch {
            formatter: ValueKind::ZonedDateTime,
            values: ValueKind::Number
        })
    );
}

#[test]
fn datetime_errors_are_returned() {
    let format = PreparedFormatter::Date(Arc::new(Chrono("%Y")));
    assert!(matches!(
        format.format_ticks(
            FormatValues::Dates(&[Some(date(2024, 1, 1)), Some(date(1800, 1, 1))]),
            TickSpacing::Uniform
        ),
        Err(FormatError::DateTime(
            DateTimeFormatError::UnsupportedValue { .. }
        ))
    ));
}
