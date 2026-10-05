//! Calendar-boundary labels through a stub provider that formats Chrono strftime patterns.

use avenger_format::{
    CalendarPatterns, DateTimeFormatError, DateTimeFormatProvider, DateTimeInputKind,
    PreparedDateFormatter, PreparedNaiveDateTimeFormatter, PreparedZonedDateTimeFormatter,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::{
    America::{Havana, New_York},
    Australia::Lord_Howe,
    Tz, UTC,
};
use std::sync::Arc;

/// Prepares strftime patterns that display zoned values in `display`, or in `year_display` for
/// a pattern of `%Y`. Date patterns reject time fields, as real providers do, and `other_months`
/// reports a calendar whose months start on other days.
#[derive(Debug)]
struct Strftime {
    display: Tz,
    year_display: Tz,
    other_months: bool,
}

impl Strftime {
    fn new(display: Tz) -> Self {
        Self {
            display,
            year_display: display,
            other_months: false,
        }
    }

    fn pattern(&self, spec: &str) -> Arc<Pattern> {
        let timezone = if spec == "%Y" {
            self.year_display
        } else {
            self.display
        };
        Arc::new(Pattern {
            spec: spec.into(),
            timezone,
        })
    }
}

#[derive(Debug)]
struct Pattern {
    spec: String,
    timezone: Tz,
}

impl DateTimeFormatProvider for Strftime {
    fn prepare_date(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        if ["%H", "%I", "%M", "%S", "%p", "f"]
            .iter()
            .any(|field| spec.contains(field))
        {
            return Err(DateTimeFormatError::UnsupportedPattern {
                input: DateTimeInputKind::Date,
                message: "time field".into(),
            });
        }
        Ok(self.pattern(spec))
    }

    fn prepare_naive(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        Ok(self.pattern(spec))
    }

    fn prepare_zoned(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
        Ok(self.pattern(spec))
    }

    fn default_calendar_patterns(&self) -> CalendarPatterns {
        patterns()
    }

    fn check_gregorian_months(&self) -> Result<(), DateTimeFormatError> {
        if self.other_months {
            return Err(DateTimeFormatError::InvalidOption {
                option: "calendar".into(),
                message: "months start on other days".into(),
            });
        }
        Ok(())
    }
}

impl PreparedDateFormatter for Pattern {
    fn format(&self, value: NaiveDate) -> Result<String, DateTimeFormatError> {
        Ok(value.format(&self.spec).to_string())
    }
}

impl PreparedNaiveDateTimeFormatter for Pattern {
    fn format(&self, value: NaiveDateTime) -> Result<String, DateTimeFormatError> {
        Ok(value.format(&self.spec).to_string())
    }
}

impl PreparedZonedDateTimeFormatter for Pattern {
    fn format(&self, value: DateTime<Utc>) -> Result<String, DateTimeFormatError> {
        Ok(value
            .with_timezone(&self.timezone)
            .format(&self.spec)
            .to_string())
    }

    fn timezone(&self) -> Tz {
        self.timezone
    }
}

/// Vega's default patterns, in strftime syntax.
fn patterns() -> CalendarPatterns {
    CalendarPatterns {
        year: "%Y".into(),
        month: "%B".into(),
        week: "%b %d".into(),
        day: "%a %d".into(),
        hour: "%I %p".into(),
        minute: "%I:%M".into(),
        second: ":%S".into(),
        millisecond: "%.3f".into(),
    }
}

fn naive(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").unwrap()
}

fn utc(text: &str) -> DateTime<Utc> {
    naive(text).and_utc()
}

/// Each case: a civil datetime and its label with Vega's patterns.
const BOUNDARIES: [(&str, &str); 12] = [
    ("2024-01-01 00:00:00", "2024"),
    ("2024-02-01 00:00:00", "February"),
    // Quarter starts are ordinary months.
    ("2024-04-01 00:00:00", "April"),
    // Sunday.
    ("2024-03-03 00:00:00", "Mar 03"),
    ("2024-03-05 00:00:00", "Tue 05"),
    ("2024-03-05 15:00:00", "03 PM"),
    ("2024-03-05 15:15:00", "03:15"),
    ("2024-03-05 15:15:30", ":30"),
    ("2024-03-05 15:15:30.250", ".250"),
    // Sub-millisecond digits are dropped before the pattern is chosen and applied.
    ("2024-03-05 15:15:30.0004", ":30"),
    ("2024-03-05 15:15:30.2509", ".250"),
    ("2024-03-05 00:00:00.0009", "Tue 05"),
];

#[test]
fn naive_values_use_their_coarsest_boundary() {
    let format = patterns().prepare_naive(&Strftime::new(UTC)).unwrap();
    for (value, expected) in BOUNDARIES {
        assert_eq!(format.format(naive(value)).unwrap(), expected, "{value}");
    }
}

#[test]
fn zoned_values_use_their_coarsest_boundary() {
    let format = patterns().prepare_zoned(&Strftime::new(UTC)).unwrap();
    for (value, expected) in BOUNDARIES {
        assert_eq!(format.format(utc(value)).unwrap(), expected, "{value}");
    }
}

#[test]
fn dates_prepare_only_date_patterns() {
    // The stub rejects time fields for dates, so preparing them would fail.
    let format = patterns().prepare_date(&Strftime::new(UTC)).unwrap();
    for (value, expected) in BOUNDARIES
        .into_iter()
        .filter(|(value, _)| value.len() == 19)
    {
        let value = naive(value);
        if value.time() == chrono::NaiveTime::MIN {
            assert_eq!(format.format(value.date()).unwrap(), expected, "{value}");
        }
    }
}

#[test]
fn leap_days_and_dates_before_1970() {
    let format = patterns().prepare_naive(&Strftime::new(UTC)).unwrap();
    for (value, expected) in [
        ("2024-02-29 00:00:00", "Thu 29"),
        ("1969-12-28 00:00:00", "Dec 28"),
        ("1969-12-31 23:00:00", "11 PM"),
        ("1960-01-01 00:00:00", "1960"),
        ("1900-03-01 00:00:00", "March"),
    ] {
        assert_eq!(format.format(naive(value)).unwrap(), expected, "{value}");
    }
}

#[test]
fn zoned_milliseconds_truncate_toward_zero() {
    let format = patterns().prepare_zoned(&Strftime::new(UTC)).unwrap();
    // 400 µs before the epoch truncates to the epoch, as a JavaScript date does.
    let value = DateTime::from_timestamp(-1, 999_600_000).unwrap();
    assert_eq!(format.format(value).unwrap(), "1970");
    let value = DateTime::from_timestamp(-1, 750_000_000).unwrap();
    assert_eq!(format.format(value).unwrap(), ".750");
}

#[test]
fn zoned_boundaries_use_the_display_timezone() {
    let format = patterns().prepare_zoned(&Strftime::new(New_York)).unwrap();
    assert_eq!(format.timezone(), New_York);
    for (value, expected) in [
        // Midnight in New York, which is 05 AM in UTC.
        ("2024-03-05 05:00:00", "Tue 05"),
        // On the day clocks spring forward at 02:00, midnight still starts the day.
        ("2024-03-10 05:00:00", "Mar 10"),
        ("2024-03-10 07:00:00", "03 AM"),
        ("2024-11-01 04:00:00", "November"),
        ("2025-01-01 05:00:00", "2025"),
    ] {
        assert_eq!(format.format(utc(value)).unwrap(), expected, "{value}");
    }
}

#[test]
fn days_start_at_their_first_instant_across_midnight_transitions() {
    let havana = patterns().prepare_zoned(&Strftime::new(Havana)).unwrap();
    for (value, expected) in [
        // Clocks skip from 00:00 to 01:00, so 01:00 starts the day.
        ("2024-03-10 05:00:00", "Mar 10"),
        ("2024-03-10 06:00:00", "02 AM"),
        // Clocks fall back from 01:00 to 00:00: the first midnight starts the day and the
        // repeated midnight is an hour within it.
        ("2024-11-03 04:00:00", "Nov 03"),
        ("2024-11-03 05:00:00", "12 AM"),
    ] {
        assert_eq!(havana.format(utc(value)).unwrap(), expected, "{value}");
    }
    // Lord Howe is 10:30 ahead of UTC in winter.
    let lord_howe = patterns().prepare_zoned(&Strftime::new(Lord_Howe)).unwrap();
    for (value, expected) in [
        ("2024-06-04 13:30:00", "Wed 05"),
        ("2024-06-04 16:30:00", "03 AM"),
        ("2024-06-30 13:30:00", "July"),
    ] {
        assert_eq!(lord_howe.format(utc(value)).unwrap(), expected, "{value}");
    }
}

#[test]
fn builders_override_single_patterns() {
    let built = patterns()
        .with_year("y")
        .with_month("m")
        .with_week("w")
        .with_day("d")
        .with_hour("H")
        .with_minute("M")
        .with_second("S")
        .with_millisecond("L");
    let expected = CalendarPatterns {
        year: "y".into(),
        month: "m".into(),
        week: "w".into(),
        day: "d".into(),
        hour: "H".into(),
        minute: "M".into(),
        second: "S".into(),
        millisecond: "L".into(),
    };
    assert_eq!(built, expected);
    let format = patterns()
        .with_day("%d")
        .prepare_naive(&Strftime::new(UTC))
        .unwrap();
    assert_eq!(format.format(naive("2024-03-05 00:00:00")).unwrap(), "05");
    assert_eq!(
        format.format(naive("2024-03-03 00:00:00")).unwrap(),
        "Mar 03"
    );
}

#[test]
fn preparation_errors_name_the_pattern() {
    let error = patterns()
        .with_day("%a %H")
        .prepare_date(&Strftime::new(UTC))
        .unwrap_err();
    assert_eq!(
        error,
        DateTimeFormatError::UnsupportedPattern {
            input: DateTimeInputKind::Date,
            message: "`day` pattern: time field".into(),
        }
    );
}

#[test]
fn zoned_formatters_must_share_a_timezone() {
    let provider = Strftime {
        year_display: UTC,
        ..Strftime::new(New_York)
    };
    assert!(matches!(
        patterns().prepare_zoned(&provider),
        Err(DateTimeFormatError::InvalidOption { option, .. }) if option == "timezone"
    ));
    // A shared timezone is reported by the composite.
    let format = patterns().prepare_zoned(&Strftime::new(Havana)).unwrap();
    assert_eq!(format.timezone(), Havana);
    assert_eq!(
        format
            .format(
                Havana
                    .with_ymd_and_hms(2024, 7, 1, 0, 0, 0)
                    .unwrap()
                    .to_utc()
            )
            .unwrap(),
        "July"
    );
}

#[test]
fn preparation_checks_the_providers_calendar() {
    let provider = Strftime {
        other_months: true,
        ..Strftime::new(UTC)
    };
    let is_calendar_error = |error: DateTimeFormatError| matches!(error, DateTimeFormatError::InvalidOption { option, .. } if option == "calendar");
    assert!(is_calendar_error(
        patterns().prepare_date(&provider).unwrap_err()
    ));
    assert!(is_calendar_error(
        patterns().prepare_naive(&provider).unwrap_err()
    ));
    assert!(is_calendar_error(
        patterns().prepare_zoned(&provider).unwrap_err()
    ));
}
