//! Calendar-boundary labels with each provider's default patterns.

use avenger_format::{CalendarPatterns, DateTimeFormatError, DateTimeFormatProvider};
use avenger_format_datetime_icu::{
    CalendarAlgorithm, IcuPatternDateTimeFormatProvider, IcuSemanticDateTimeFormatProvider,
};
use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::America::New_York;

fn naive(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").unwrap()
}

const VALUES: [&str; 8] = [
    "2024-01-01 00:00:00",
    "2024-02-01 00:00:00",
    "2024-03-03 00:00:00",
    "2024-03-05 00:00:00",
    "2024-03-05 15:00:00",
    "2024-03-05 15:15:00",
    "2024-03-05 15:15:30",
    "2024-03-05 15:15:30.250",
];

/// Label each value as a naive datetime, as a zoned datetime in New York, and at midnight as a
/// date.
fn assert_labels(
    patterns: &CalendarPatterns,
    provider: &dyn DateTimeFormatProvider,
    new_york: &dyn DateTimeFormatProvider,
    expected: [&str; 8],
) {
    let dates = patterns.prepare_date(provider).unwrap();
    let naive_format = patterns.prepare_naive(provider).unwrap();
    let zoned = patterns.prepare_zoned(new_york).unwrap();
    for (value, expected) in VALUES.iter().zip(expected) {
        let value = naive(value);
        assert_eq!(
            naive_format.format(value).unwrap(),
            expected,
            "{provider:?} {value}"
        );
        let instant = New_York.from_local_datetime(&value).unwrap().to_utc();
        assert_eq!(
            zoned.format(instant).unwrap(),
            expected,
            "{provider:?} {value}"
        );
        if value.time() == chrono::NaiveTime::MIN {
            assert_eq!(
                dates.format(value.date()).unwrap(),
                expected,
                "{provider:?} {value}"
            );
        }
    }
}

#[test]
fn semantic_labels_are_localized_by_boundary() {
    for (locale, expected) in [
        (
            "en-US",
            [
                "2024",
                "February",
                "Mar 3",
                "5 Tue",
                "3\u{202f}PM",
                "3:15\u{202f}PM",
                "3:15:30\u{202f}PM",
                "3:15:30.250\u{202f}PM",
            ],
        ),
        (
            "de-DE",
            [
                "2024",
                "Februar",
                "03.03.",
                "Di., 5.",
                "15 Uhr",
                "15:15",
                "15:15:30",
                "15:15:30,250",
            ],
        ),
    ] {
        let provider = IcuSemanticDateTimeFormatProvider::new().with_locale(locale);
        let patterns = provider.default_calendar_patterns();
        let new_york = provider.clone().with_timezone(New_York);
        assert_labels(&patterns, &provider, &new_york, expected);
    }
}

#[test]
fn pattern_labels_use_the_locales_names() {
    for (locale, expected) in [
        (
            "en-US",
            [
                "2024", "February", "Mar 3", "Tue 5", "3 PM", "3:15", ":30", ":30.250",
            ],
        ),
        (
            "de-DE",
            [
                "2024", "Februar", "März 3", "Di. 5", "3 PM", "3:15", ":30", ":30,250",
            ],
        ),
    ] {
        let provider = IcuPatternDateTimeFormatProvider::new().with_locale(locale);
        let patterns = provider.default_calendar_patterns();
        let new_york = provider.clone().with_timezone(New_York);
        assert_labels(&patterns, &provider, &new_york, expected);
    }
}

/// Label Jan 1, 2024 with calendar patterns prepared through a provider.
fn new_year(patterns: CalendarPatterns, provider: &dyn DateTimeFormatProvider) -> String {
    patterns
        .prepare_naive(provider)
        .unwrap()
        .format(naive("2024-01-01 00:00:00"))
        .unwrap()
}

#[test]
fn calendars_need_gregorian_months() {
    // Buddhist and Japanese years count from other eras, but months match.
    let thai = IcuSemanticDateTimeFormatProvider::new().with_locale("th-TH");
    assert_eq!(
        new_year(thai.default_calendar_patterns(), &thai),
        "พ.ศ. 2567"
    );
    let thai = IcuPatternDateTimeFormatProvider::new().with_locale("th-TH");
    assert_eq!(new_year(thai.default_calendar_patterns(), &thai), "2567");
    let japanese = IcuPatternDateTimeFormatProvider::new().with_locale("ja-JP-u-ca-japanese");
    assert_eq!(
        new_year(japanese.default_calendar_patterns(), &japanese),
        "6"
    );
    let with_era = japanese.default_calendar_patterns().with_year("G y");
    assert_eq!(new_year(with_era, &japanese), "令和 6");

    // Preparing calendar patterns rejects calendars whose months start on other days.
    let hebrew = IcuSemanticDateTimeFormatProvider::new().with_locale("en-US-u-ca-hebrew");
    let providers: [&dyn DateTimeFormatProvider; 4] = [
        &hebrew,
        &IcuSemanticDateTimeFormatProvider::new().with_calendar(CalendarAlgorithm::Persian),
        &IcuPatternDateTimeFormatProvider::new().with_locale("en-US-u-ca-hebrew"),
        &IcuPatternDateTimeFormatProvider::new().with_calendar(CalendarAlgorithm::Persian),
    ];
    for provider in providers {
        assert!(
            matches!(
                provider.default_calendar_patterns().prepare_naive(provider),
                Err(DateTimeFormatError::InvalidOption { option, .. }) if option == "calendar"
            ),
            "{provider:?}"
        );
    }
    // Single patterns still format in those calendars.
    assert!(hebrew
        .prepare_date("{dateFields=month dateLength=long}")
        .is_ok());
}
