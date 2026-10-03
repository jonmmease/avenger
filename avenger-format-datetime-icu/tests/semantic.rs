use avenger_format::{DateTimeFormatError, DateTimeFormatProvider, DateTimeInputKind};
use avenger_format_datetime_icu::{CalendarAlgorithm, IcuSemanticDateTimeFormatProvider};
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::{America::New_York, Asia::Tokyo};

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

#[test]
fn literals_and_option_whitespace_are_preserved() {
    let provider = IcuSemanticDateTimeFormatProvider::new();
    for (spec, expected) in [
        ("{dateFields=year}", "2024"),
        ("Today's year: {dateFields=year}", "Today's year: 2024"),
        ("{dateFields=year} onwards", "2024 onwards"),
        (
            "  année \\{reported\\} \\\\ {dateFields=year} — \"ok\"  ",
            "  année {reported} \\ 2024 — \"ok\"  ",
        ),
    ] {
        assert_eq!(
            provider
                .prepare_date(spec)
                .unwrap()
                .format(date(2024, 3, 11))
                .unwrap(),
            expected,
            "{spec}"
        );
    }
    for spec in [
        "{dateFields=year-month-day dateLength=long}",
        "{\n dateLength = long\tdateFields= year-month-day \r\n}",
    ] {
        assert_eq!(
            provider
                .prepare_date(spec)
                .unwrap()
                .format(date(2024, 3, 11))
                .unwrap(),
            "March 11, 2024"
        );
    }
}

#[test]
fn syntax_errors_report_byte_positions() {
    let provider = IcuSemanticDateTimeFormatProvider::new();
    for (spec, position) in [
        ("", 0),
        ("yMMMdjm", 7),
        ("dateFields=year", 15),
        ("{}", 0),
        ("{dateFields=year", 16),
        ("{dateFields=}", 12),
        ("é {dateFields year}", 15),
        ("é \\q {dateFields=year}", 3),
        ("{dateFields=year}\\", 17),
        ("{dateFields=year}{dateFields=month}", 17),
        ("{{dateFields=year}}", 1),
        ("{dateFields={year}}", 12),
        ("{dateFields=year}}", 17),
        ("{dateFields=year dateFields=month}", 17),
        ("{$value :datetime dateFields=year}", 1),
    ] {
        assert!(
            matches!(
                provider.prepare_date(spec).unwrap_err(),
                DateTimeFormatError::InvalidPattern { position: Some(actual), .. } if actual == position
            ),
            "{spec}: expected byte {position}"
        );
    }
}

#[test]
fn option_errors_and_input_mismatches_fail_during_preparation() {
    let provider = IcuSemanticDateTimeFormatProvider::new();
    for (spec, name) in [
        ("{fields=year}", "fields"),
        ("{dateFields=month-year}", "dateFields"),
        ("{dateFields=year-day}", "dateFields"),
        ("{dateFields=year dateLength=full}", "dateLength"),
        ("{dateLength=long}", "dateLength"),
        ("{timePrecision=millisecond}", "timePrecision"),
        ("{timePrecision=hour hour12=yes}", "hour12"),
        ("{hour12=false}", "hour12"),
        ("{timeZoneStyle=short}", "timeZoneStyle"),
        (
            "{timePrecision=minute timeZoneStyle=narrow}",
            "timeZoneStyle",
        ),
        (
            "{timePrecision=second fractionalSecondDigits=0}",
            "fractionalSecondDigits",
        ),
        (
            "{timePrecision=second fractionalSecondDigits=10}",
            "fractionalSecondDigits",
        ),
        (
            "{timePrecision=minute fractionalSecondDigits=3}",
            "fractionalSecondDigits",
        ),
        ("{calendar=hebrew}", "calendar"),
        ("{locale=en-US}", "locale"),
        ("{timeZone=UTC}", "timeZone"),
    ] {
        assert!(
            matches!(
                provider.prepare_zoned(spec).unwrap_err(),
                DateTimeFormatError::InvalidOption { option, .. } if option == name
            ),
            "{spec}"
        );
    }
    assert!(matches!(
        provider.prepare_date("{timePrecision=minute}").unwrap_err(),
        DateTimeFormatError::UnsupportedPattern {
            input: DateTimeInputKind::Date,
            ..
        }
    ));
    for spec in [
        "{timePrecision=minute timeZoneStyle=short}",
        "{dateFields=year timePrecision=hour}",
        "{dateFields=month timePrecision=minute}",
        "{dateFields=year-month timePrecision=second}",
    ] {
        assert!(
            matches!(
                provider.prepare_naive(spec).unwrap_err(),
                DateTimeFormatError::UnsupportedPattern {
                    input: DateTimeInputKind::Naive,
                    ..
                }
            ),
            "{spec}"
        );
    }
}

#[test]
fn chart_date_fields_and_lengths_select_localized_layouts() {
    let provider = IcuSemanticDateTimeFormatProvider::new();
    for (spec, expected) in [
        ("{dateFields=day}", "11"),
        ("{dateFields=month dateLength=long}", "March"),
        ("{dateFields=year-month}", "Mar 2024"),
        ("{dateFields=year-month-day dateLength=short}", "3/11/2024"),
        ("{dateFields=year-month-day-weekday}", "Mon, Mar 11, 2024"),
    ] {
        assert_eq!(
            provider
                .prepare_date(spec)
                .unwrap()
                .format(date(2024, 3, 11))
                .unwrap(),
            expected,
            "{spec}"
        );
    }
    // One prepared formatter must select the era for each value.
    let formatter = provider
        .prepare_date("{dateFields=year-month-day}")
        .unwrap();
    for (year, expected) in [(2024, "Mar 11, 2024"), (-43, "Mar 11, 44 BC")] {
        assert_eq!(formatter.format(date(year, 3, 11)).unwrap(), expected);
    }
}

#[test]
fn prepared_formatters_retain_locale_layout_and_hour_cycle() {
    let value = date(2024, 3, 11).and_hms_opt(15, 30, 45).unwrap();
    for (locale, expected) in [
        ("en-US", "As of Mar 11, 2024, 3:30\u{202f}PM (estimated)"),
        ("fr_FR", "As of 11 mars 2024, 15:30 (estimated)"),
    ] {
        let provider = IcuSemanticDateTimeFormatProvider::new()
            .with_locale(locale)
            .with_timezone(Tokyo);
        let formatter = provider
            .prepare_naive("As of {dateFields=year-month-day timePrecision=minute} (estimated)")
            .unwrap();
        drop(provider);
        assert_eq!(formatter.format(value).unwrap(), expected);
    }
    for (locale, hour12, expected) in [
        ("en-US", "false", "15:30"),
        ("fr-FR", "true", "3:30\u{202f}PM"),
    ] {
        let formatter = IcuSemanticDateTimeFormatProvider::new()
            .with_locale(locale)
            .prepare_naive(&format!("{{timePrecision=minute hour12={hour12}}}"))
            .unwrap();
        assert_eq!(formatter.format(value).unwrap(), expected);
    }
}

#[test]
fn time_precision_preserves_fractional_and_leap_seconds() {
    let provider = IcuSemanticDateTimeFormatProvider::new();
    let value = date(2024, 3, 11)
        .and_hms_nano_opt(15, 30, 45, 123_456_789)
        .unwrap();
    for (spec, expected) in [
        ("{timePrecision=hour hour12=false}", "15"),
        ("{timePrecision=second hour12=false}", "15:30:45"),
        (
            "{timePrecision=second fractionalSecondDigits=9 hour12=false}",
            "15:30:45.123456789",
        ),
    ] {
        assert_eq!(
            provider.prepare_naive(spec).unwrap().format(value).unwrap(),
            expected
        );
    }
    let leap = date(2016, 12, 31)
        .and_hms_nano_opt(23, 59, 59, 1_500_000_000)
        .unwrap();
    assert_eq!(
        provider
            .prepare_naive("{timePrecision=second fractionalSecondDigits=3 hour12=false}")
            .unwrap()
            .format(leap)
            .unwrap(),
        "23:59:60.500"
    );
}

#[test]
fn calendars_use_local_dates_and_preserve_leap_months() {
    let provider = IcuSemanticDateTimeFormatProvider::new().with_locale("en-US-u-ca-hebrew");
    let formatter = provider
        .prepare_date("{dateFields=year-month-day dateLength=long}")
        .unwrap();
    for (value, expected) in [
        (date(2024, 2, 10), "1 Adar I 5784"),
        (date(2024, 3, 11), "1 Adar II 5784"),
    ] {
        assert_eq!(formatter.format(value).unwrap(), expected);
    }
    let provider = provider
        .with_calendar(CalendarAlgorithm::Japanese)
        .with_timezone(Tokyo);
    let spec = "{dateFields=year-month-day}";
    let value = date(2019, 4, 30).and_hms_opt(15, 0, 0).unwrap();
    assert_eq!(
        provider
            .prepare_date(spec)
            .unwrap()
            .format(value.date())
            .unwrap(),
        "Apr 30, 31 Heisei"
    );
    assert_eq!(
        provider.prepare_naive(spec).unwrap().format(value).unwrap(),
        "Apr 30, 31 Heisei"
    );
    assert_eq!(
        provider
            .prepare_zoned(spec)
            .unwrap()
            .format(value.and_utc())
            .unwrap(),
        "May 1, 1 Reiwa"
    );
}

#[test]
fn zoned_formatting_uses_per_value_offsets_and_names() {
    let provider = IcuSemanticDateTimeFormatProvider::new().with_timezone(New_York);
    let formatter = provider
        .prepare_zoned("{dateFields=year-month-day timePrecision=minute timeZoneStyle=short}")
        .unwrap();
    for (timestamp, expected) in [
        (1_710_053_940, "Mar 10, 2024, 1:59\u{202f}AM EST"),
        (1_710_054_000, "Mar 10, 2024, 3:00\u{202f}AM EDT"),
    ] {
        assert_eq!(
            formatter
                .format(DateTime::from_timestamp(timestamp, 0).unwrap())
                .unwrap(),
            expected
        );
    }
    let value = DateTime::from_timestamp(1_710_054_000, 0).unwrap();
    for (style, expected) in [
        ("long", "03:00 Eastern Daylight Time"),
        ("shortGeneric", "03:00 ET"),
        ("longGeneric", "03:00 Eastern Time"),
        ("shortOffset", "03:00 GMT-4"),
        ("longOffset", "03:00 GMT-04:00"),
    ] {
        let formatter = provider
            .prepare_zoned(&format!(
                "{{timePrecision=minute hour12=false timeZoneStyle={style}}}"
            ))
            .unwrap();
        assert_eq!(formatter.format(value).unwrap(), expected, "{style}");
    }
    let formatter = provider
        .with_timezone(Tokyo)
        .prepare_zoned("{dateFields=year}")
        .unwrap();
    assert_eq!(
        formatter.format(DateTime::<Utc>::MAX_UTC).unwrap_err(),
        DateTimeFormatError::OutOfRange
    );
}

#[test]
fn settings_are_validated_and_serialized() {
    let provider = IcuSemanticDateTimeFormatProvider::new()
        .with_locale("en-US")
        .with_calendar(CalendarAlgorithm::Hebrew)
        .with_timezone(Tokyo);
    let json = serde_json::to_string(&provider).unwrap();
    assert_eq!(
        serde_json::from_str::<IcuSemanticDateTimeFormatProvider>(&json).unwrap(),
        provider
    );
    assert!(matches!(
        provider
            .with_locale("en-u-ca-unknown")
            .prepare_date("{dateFields=year}")
            .unwrap_err(),
        DateTimeFormatError::LocaleUnavailable { .. }
    ));
    for provider in [
        IcuSemanticDateTimeFormatProvider::new().with_calendar(CalendarAlgorithm::Iso8601),
        IcuSemanticDateTimeFormatProvider::new().with_locale("en-u-ca-iso8601"),
    ] {
        let error = provider.prepare_date("{dateFields=year}").unwrap_err();
        if provider.calendar.is_some() {
            assert!(
                matches!(error, DateTimeFormatError::InvalidOption { option, .. } if option == "calendar")
            );
        } else {
            assert!(matches!(
                error,
                DateTimeFormatError::LocaleUnavailable { .. }
            ));
        }
    }
}
