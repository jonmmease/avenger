//! Calendar-boundary labels with the default strftime patterns.

use avenger_format::DateTimeFormatProvider;
use avenger_format_datetime_chrono::ChronoDateTimeFormatProvider;
use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::America::New_York;

fn naive(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").unwrap()
}

#[test]
fn values_use_their_coarsest_boundary() {
    let provider = ChronoDateTimeFormatProvider::new();
    let patterns = provider.default_calendar_patterns();
    let dates = patterns.prepare_date(&provider).unwrap();
    let naive_format = patterns.prepare_naive(&provider).unwrap();
    let zoned = patterns
        .prepare_zoned(&provider.with_timezone(New_York))
        .unwrap();
    for (value, expected) in [
        ("2024-01-01 00:00:00", "2024"),
        ("2024-02-01 00:00:00", "February"),
        ("2024-03-03 00:00:00", "Mar 03"),
        ("2024-03-05 00:00:00", "Tue 05"),
        ("2024-03-05 15:00:00", "03 PM"),
        ("2024-03-05 15:15:00", "03:15"),
        ("2024-03-05 15:15:30", ":30"),
        ("2024-03-05 15:15:30.250", ".250"),
    ] {
        let value = naive(value);
        assert_eq!(naive_format.format(value).unwrap(), expected, "{value}");
        let instant = New_York.from_local_datetime(&value).unwrap().to_utc();
        assert_eq!(zoned.format(instant).unwrap(), expected, "{value}");
        if value.time() == chrono::NaiveTime::MIN {
            assert_eq!(dates.format(value.date()).unwrap(), expected, "{value}");
        }
    }
}

/// Locales without AM/PM markers leave `%p` empty, so 24-hour patterns read better there.
#[cfg(feature = "all-locales")]
#[test]
fn labels_follow_the_locale() {
    let provider = ChronoDateTimeFormatProvider::new().with_locale("de_DE");
    let default = provider
        .default_calendar_patterns()
        .prepare_naive(&provider)
        .unwrap();
    assert_eq!(default.format(naive("2024-03-05 15:00:00")).unwrap(), "03 ");
    let format = provider
        .default_calendar_patterns()
        .with_hour("%H:00")
        .with_minute("%H:%M")
        .prepare_naive(&provider)
        .unwrap();
    for (value, expected) in [
        ("2024-02-01 00:00:00", "Februar"),
        ("2024-03-03 00:00:00", "Mär 03"),
        ("2024-03-05 00:00:00", "Di 05"),
        ("2024-03-05 15:00:00", "15:00"),
        ("2024-03-05 15:15:00", "15:15"),
        ("2024-03-05 15:15:30.250", ",250"),
    ] {
        assert_eq!(format.format(naive(value)).unwrap(), expected, "{value}");
    }
}
