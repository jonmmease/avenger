use avenger_format::{DateTimeFormatError, DateTimeFormatProvider, DateTimeInputKind};
use avenger_format_datetime_icu::{
    CalendarAlgorithm, HijriCalendarAlgorithm, IcuPatternDateTimeFormatProvider,
};
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::{America::New_York, Asia::Tokyo};

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

#[test]
fn prepared_formatters_retain_settings_and_preserve_inputs() {
    let provider = IcuPatternDateTimeFormatProvider::new().with_timezone(New_York);
    let date_formatter = provider.prepare_date("yyyy-MM-dd").unwrap();
    let naive = provider
        .prepare_naive("yyyy-MM-dd HH:mm:ss.SSSSSSSSS")
        .unwrap();
    let zoned = provider
        .prepare_zoned("yyyy-MM-dd HH:mm:ss.SSSSSSSSS XXX")
        .unwrap();
    drop(provider);

    let value = date(2024, 2, 29)
        .and_hms_nano_opt(0, 0, 0, 123_456_789)
        .unwrap();
    assert_eq!(date_formatter.format(value.date()).unwrap(), "2024-02-29");
    assert_eq!(
        naive.format(value).unwrap(),
        "2024-02-29 00:00:00.123456789"
    );
    assert_eq!(
        zoned.format(value.and_utc()).unwrap(),
        "2024-02-28 19:00:00.123456789 -05:00"
    );
}

#[test]
fn preparation_validates_fields_and_respects_literals() {
    let provider = IcuPatternDateTimeFormatProvider::new();
    for pattern in ["HH:mm", "XXX", "Z", "w", "MMM MMMM"] {
        assert!(
            matches!(
                provider.prepare_date(pattern).unwrap_err(),
                DateTimeFormatError::UnsupportedPattern {
                    input: DateTimeInputKind::Date,
                    ..
                }
            ),
            "{pattern}"
        );
    }
    for pattern in ["XXX", "Z", "SSS", "ss..SSS", "'ss'SSS"] {
        assert!(
            matches!(
                provider.prepare_naive(pattern).unwrap_err(),
                DateTimeFormatError::UnsupportedPattern {
                    input: DateTimeInputKind::Naive,
                    ..
                }
            ),
            "{pattern}"
        );
    }
    assert!(matches!(
        provider.prepare_date("'unclosed").unwrap_err(),
        DateTimeFormatError::InvalidPattern { position: None, .. }
    ));
    for (pattern, expected) in [("", ""), ("'HH:mm Z w' '' yyyy", "HH:mm Z w ' 2024")] {
        assert_eq!(
            provider
                .prepare_date(pattern)
                .unwrap()
                .format(date(2024, 2, 29))
                .unwrap(),
            expected
        );
    }
}

#[test]
fn unavailable_locales_fail_during_preparation() {
    for locale in ["xx", "frr-FR"] {
        let provider = IcuPatternDateTimeFormatProvider::new().with_locale(locale);
        for error in [
            provider.prepare_date("MMM d, y").err(),
            provider.prepare_naive("yyyy-MM-dd HH:mm").err(),
            provider.prepare_zoned("yyyy-MM-dd HH:mm XXX").err(),
        ] {
            assert!(
                matches!(&error, Some(DateTimeFormatError::LocaleUnavailable { locale: actual, .. }) if actual == locale),
                "{locale}: {error:?}"
            );
        }
    }
}

#[test]
fn settings_are_validated_and_serialized() {
    for locale in ["fr-FR", "fr_FR", "fr-XX"] {
        let formatter = IcuPatternDateTimeFormatProvider::new()
            .with_locale(locale)
            .prepare_date("MMMM")
            .unwrap();
        assert_eq!(formatter.format(date(2024, 3, 1)).unwrap(), "mars");
    }
    for locale in ["en!US", "en-US-u-ca-unknown"] {
        assert!(matches!(
            IcuPatternDateTimeFormatProvider::new()
                .with_locale(locale)
                .with_calendar(CalendarAlgorithm::Hebrew)
                .prepare_date("y")
                .unwrap_err(),
            DateTimeFormatError::LocaleUnavailable { .. }
        ));
    }
    assert!(matches!(
        IcuPatternDateTimeFormatProvider::new().with_calendar(CalendarAlgorithm::Iso8601)
            .prepare_date("y").unwrap_err(),
        DateTimeFormatError::InvalidOption { option, .. } if option == "calendar"
    ));
    assert!(matches!(
        IcuPatternDateTimeFormatProvider::new()
            .with_locale("en-u-ca-iso8601")
            .prepare_date("y")
            .unwrap_err(),
        DateTimeFormatError::LocaleUnavailable { .. }
    ));
    let arabic = IcuPatternDateTimeFormatProvider::new()
        .with_locale("en-US-u-nu-arab")
        .prepare_date("yyyy-MM-dd")
        .unwrap();
    assert_eq!(arabic.format(date(2024, 2, 29)).unwrap(), "٢٠٢٤-٠٢-٢٩");

    for (calendar, identifier) in [
        (CalendarAlgorithm::Hebrew, "hebrew"),
        (
            CalendarAlgorithm::Hijri(Some(HijriCalendarAlgorithm::Umalqura)),
            "islamic-umalqura",
        ),
    ] {
        let provider = IcuPatternDateTimeFormatProvider::new()
            .with_calendar(calendar)
            .with_timezone(Tokyo);
        let value = serde_json::to_value(&provider).unwrap();
        assert_eq!(value["calendar"], identifier);
        assert_eq!(
            serde_json::from_value::<IcuPatternDateTimeFormatProvider>(value).unwrap(),
            provider
        );
    }
    for json in ["{}", r#"{"calendar":null}"#] {
        assert_eq!(
            serde_json::from_str::<IcuPatternDateTimeFormatProvider>(json).unwrap(),
            IcuPatternDateTimeFormatProvider::new()
        );
    }
    assert!(
        serde_json::from_str::<IcuPatternDateTimeFormatProvider>(r#"{"calendar":"unknown"}"#)
            .is_err()
    );
}

#[test]
fn zoned_formatting_uses_per_value_offsets_and_names() {
    let formatter = IcuPatternDateTimeFormatProvider::new()
        .with_timezone(New_York)
        .prepare_zoned("yyyy-MM-dd HH:mm XXX zzzz")
        .unwrap();
    for (timestamp, expected) in [
        (
            1_710_053_940,
            "2024-03-10 01:59 -05:00 Eastern Standard Time",
        ),
        (
            1_710_054_000,
            "2024-03-10 03:00 -04:00 Eastern Daylight Time",
        ),
    ] {
        assert_eq!(
            formatter
                .format(DateTime::from_timestamp(timestamp, 0).unwrap())
                .unwrap(),
            expected
        );
    }
}

#[test]
fn calendar_selection_obeys_precedence_and_era_boundaries() {
    for provider in [
        IcuPatternDateTimeFormatProvider::new().with_calendar(CalendarAlgorithm::Buddhist),
        IcuPatternDateTimeFormatProvider::new().with_locale("th-TH"),
    ] {
        assert_eq!(
            provider
                .prepare_date("yyyy-MM-dd")
                .unwrap()
                .format(date(2024, 2, 29))
                .unwrap(),
            "2567-02-29"
        );
    }
    for provider in [
        IcuPatternDateTimeFormatProvider::new()
            .with_locale("en-US-u-ca-buddhist")
            .with_calendar(CalendarAlgorithm::Hebrew),
        IcuPatternDateTimeFormatProvider::new().with_locale("en-US-u-ca-hebrew"),
    ] {
        assert_eq!(
            provider
                .prepare_date("d MMMM y")
                .unwrap()
                .format(date(2024, 3, 11))
                .unwrap(),
            "1 Adar II 5784"
        );
    }
    let japanese = IcuPatternDateTimeFormatProvider::new()
        .with_calendar(CalendarAlgorithm::Japanese)
        .prepare_date("G y-MM-dd")
        .unwrap();
    for (value, expected) in [
        (date(2019, 4, 30), "Heisei 31-04-30"),
        (date(2019, 5, 1), "Reiwa 1-05-01"),
    ] {
        assert_eq!(japanese.format(value).unwrap(), expected);
    }
}

#[test]
fn hebrew_calendar_formats_distinct_leap_months() {
    let formatter = IcuPatternDateTimeFormatProvider::new()
        .with_calendar(CalendarAlgorithm::Hebrew)
        .prepare_date("d MMMM y")
        .unwrap();
    for (value, expected) in [
        (date(2024, 2, 10), "1 Adar I 5784"),
        (date(2024, 3, 11), "1 Adar II 5784"),
    ] {
        assert_eq!(formatter.format(value).unwrap(), expected);
    }
}

#[test]
fn zoned_calendar_conversion_uses_the_local_date() {
    let provider = IcuPatternDateTimeFormatProvider::new()
        .with_calendar(CalendarAlgorithm::Japanese)
        .with_timezone(Tokyo);
    let value = date(2019, 4, 30).and_hms_opt(15, 0, 0).unwrap();
    assert_eq!(
        provider
            .prepare_zoned("G y-MM-dd")
            .unwrap()
            .format(value.and_utc())
            .unwrap(),
        "Reiwa 1-05-01"
    );
    assert_eq!(
        provider
            .prepare_naive("G y-MM-dd")
            .unwrap()
            .format(value)
            .unwrap(),
        "Heisei 31-04-30"
    );
}

#[test]
fn month_names_follow_pattern_context() {
    let provider = IcuPatternDateTimeFormatProvider::new().with_locale("pl-PL");
    for (pattern, expected) in [("d MMMM", "1 marca"), ("LLLL", "marzec")] {
        assert_eq!(
            provider
                .prepare_date(pattern)
                .unwrap()
                .format(date(2024, 3, 1))
                .unwrap(),
            expected
        );
    }
}

#[test]
fn conversion_preserves_leap_seconds_and_checks_range() {
    let provider = IcuPatternDateTimeFormatProvider::new();
    let leap = date(2016, 12, 31)
        .and_hms_nano_opt(23, 59, 59, 1_500_000_000)
        .unwrap();
    assert_eq!(
        provider
            .prepare_naive("HH:mm:ss.SSS")
            .unwrap()
            .format(leap)
            .unwrap(),
        "23:59:60.500"
    );
    assert_eq!(
        provider
            .prepare_zoned("HH:mm:ss.SSS XXX")
            .unwrap()
            .format(leap.and_utc())
            .unwrap(),
        "23:59:60.500 Z"
    );
    assert_eq!(
        provider
            .prepare_date("y-MM-dd")
            .unwrap()
            .format(date(12000, 2, 29))
            .unwrap(),
        "12000-02-29"
    );
    let formatter = provider
        .with_timezone(Tokyo)
        .prepare_zoned("yyyy-MM-dd HH:mm")
        .unwrap();
    assert_eq!(
        formatter.format(DateTime::<Utc>::MAX_UTC).unwrap_err(),
        DateTimeFormatError::OutOfRange
    );
}
