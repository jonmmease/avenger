use avenger_format_datetime_d3::{
    D3DateTimeFormatConfig, DateTimeFormatContext, DateTimeFormatError, DateTimeLocaleRegistry,
    DateTimeLocaleSpec, DateTimeParseError, NaiveDateTimeInput, PreparedDateTimeFormat,
    ResolvedDateTimeLocale,
};
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::{America::New_York, Asia::Tokyo, UTC};

#[test]
fn civil_values_preserve_fields() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, New_York);
    let scalar = PreparedDateTimeFormat::new(Some("%Y-%m-%d %H:%M:%S.%L %f"), context).unwrap();
    let date = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
    assert_eq!(
        scalar.format_naive(NaiveDateTimeInput::Date(date)).unwrap(),
        "2024-02-29 00:00:00.000 000000"
    );
    assert_eq!(
        scalar
            .format_naive(NaiveDateTimeInput::DateTime(
                date.and_hms_micro_opt(13, 5, 6, 7_999).unwrap()
            ))
            .unwrap(),
        "2024-02-29 13:05:06.007 007000"
    );
}

#[test]
fn civil_preparation_rejects_instant_fields_including_locale_expansions() {
    use avenger_format::DateTimeFormatProvider;
    let provider = avenger_format_datetime_d3::D3DateTimeFormatProvider;
    for directive in ["%Z", "%Q", "%s"] {
        let config = D3DateTimeFormatConfig::new()
            .with_locale("custom")
            .with_custom_locale(
                "custom",
                DateTimeLocaleSpec {
                    time: directive.into(),
                    ..Default::default()
                },
            );
        for spec in [directive, "%c"] {
            assert!(provider
                .prepare_naive(&config, spec)
                .unwrap_err()
                .to_string()
                .contains(directive));
            assert!(provider.prepare_zoned(&config, spec).is_ok());
        }
    }
}

#[test]
fn display_timezone_preserves_epoch() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, Tokyo);
    let format = PreparedDateTimeFormat::new(Some("%Y-%m-%d %H:%M %Z %Q %s"), context).unwrap();
    assert_eq!(
        format.format_zoned(DateTime::UNIX_EPOCH).unwrap(),
        "1970-01-01 09:00 +0900 0 0"
    );
}

#[test]
fn locale_registration_validates_and_preserves_prepared_formats() {
    let mut registry = DateTimeLocaleRegistry::with_builtins();
    assert_eq!(
        registry.resolve("fr-FR"),
        Err(DateTimeFormatError::LocaleNotFound("fr-FR".into()))
    );
    registry
        .register_custom_locale_json("fr-FR", include_str!("fixtures/locales/fr-FR.json"))
        .unwrap();
    let locale = registry.resolve("fr-FR").unwrap();
    let prepare = |locale: &ResolvedDateTimeLocale| {
        PreparedDateTimeFormat::new(Some("%B %x"), DateTimeFormatContext::new(locale, UTC)).unwrap()
    };
    let original = prepare(&locale);
    let mut replacement = locale.definition().clone();
    replacement.months[0] = "Updated".into();
    replacement.date = "%Y".into();
    registry
        .register_custom_locale("fr-FR", replacement.clone())
        .unwrap();
    let date = NaiveDateTimeInput::Date(NaiveDate::from_ymd_opt(2024, 1, 2).unwrap());
    assert_eq!(original.format_naive(date).unwrap(), "janvier 02/01/2024");
    assert_eq!(
        prepare(&registry.resolve("fr-FR").unwrap())
            .format_naive(date)
            .unwrap(),
        "Updated 2024"
    );

    let mut invalid = serde_json::to_value(replacement).unwrap();
    invalid["months"].as_array_mut().unwrap().pop();
    assert!(matches!(
        registry.register_custom_locale_json("fr-FR", &invalid.to_string()),
        Err(DateTimeFormatError::InvalidLocaleData(_))
    ));
    assert_eq!(
        prepare(&registry.resolve("fr-FR").unwrap())
            .format_naive(date)
            .unwrap(),
        "Updated 2024"
    );
}

#[test]
fn recursive_locale_patterns_are_rejected() {
    for (date_time, date) in [("%c", "%Y"), ("%x", "%c")] {
        let definition = DateTimeLocaleSpec {
            date_time: date_time.into(),
            date: date.into(),
            ..Default::default()
        };
        assert!(matches!(
            ResolvedDateTimeLocale::new("cycle", definition),
            Err(DateTimeFormatError::InvalidLocaleData(_))
        ));
    }
}

#[test]
fn malformed_patterns_report_byte_positions() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, UTC);
    for (pattern, expected_position) in [("%", 0), ("%_", 0), ("%k", 0), ("é%k", 2)] {
        assert!(matches!(
            PreparedDateTimeFormat::new(Some(pattern), context),
            Err(DateTimeFormatError::Parse(DateTimeParseError::Invalid { position, .. }))
                if position == expected_position
        ));
    }
    let date = NaiveDateTimeInput::Date(NaiveDate::MIN);
    for pattern in ["", "literal é"] {
        assert_eq!(
            PreparedDateTimeFormat::new(Some(pattern), context)
                .unwrap()
                .format_naive(date)
                .unwrap(),
            pattern
        );
    }
}

#[test]
fn out_of_range_display_dates_return_errors() {
    let locale = ResolvedDateTimeLocale::en_us();
    let last_leap = NaiveDate::MAX
        .and_hms_nano_opt(23, 59, 59, 1_500_000_000)
        .unwrap()
        .and_utc();
    for (value, zone) in [
        (DateTime::<Utc>::MAX_UTC, Tokyo),
        (DateTime::<Utc>::MIN_UTC, New_York),
        (last_leap, UTC),
    ] {
        let context = DateTimeFormatContext::new(&locale, zone);
        let scalar = PreparedDateTimeFormat::new(Some("%Y-%m-%d"), context).unwrap();
        assert_eq!(
            scalar.format_zoned(value),
            Err(DateTimeFormatError::DateTimeOutOfRange)
        );
    }
}

#[test]
fn civil_leap_seconds_are_rejected() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, UTC);
    let scalar = PreparedDateTimeFormat::new(Some("%H:%M:%S.%L"), context).unwrap();
    let value = NaiveDateTimeInput::DateTime(
        NaiveDate::from_ymd_opt(2016, 12, 31)
            .unwrap()
            .and_hms_milli_opt(23, 59, 59, 1500)
            .unwrap(),
    );
    assert_eq!(
        scalar.format_naive(value),
        Err(DateTimeFormatError::LeapSecondForNaive)
    );
}

#[test]
fn submillisecond_instants_use_javascript_date_precision() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, UTC);
    let format = PreparedDateTimeFormat::new(Some("%Q %s %L %f"), context).unwrap();
    let just_before_epoch = DateTime::from_timestamp(-1, 999_500_000).unwrap();
    for (value, expected) in [
        (just_before_epoch, "0 0 000 000000"),
        (
            DateTime::from_timestamp(-1, 998_500_000).unwrap(),
            "-1 -1 999 999000",
        ),
        (
            DateTime::from_timestamp(0, 500_000).unwrap(),
            "0 0 000 000000",
        ),
        (
            DateTime::from_timestamp(-1, 1_500_500_000).unwrap(),
            "500 0 500 500000",
        ),
    ] {
        assert_eq!(format.format_zoned(value).unwrap(), expected);
    }
}

#[test]
fn ordinal_uses_calendar_date_after_midnight_offset_change() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, chrono_tz::Asia::Kathmandu);
    let format = PreparedDateTimeFormat::new(Some("%Y-%m-%d %j"), context).unwrap();
    // D3's elapsed-day calculation gives 001 after the January 1, 1986 midnight gap.
    let value = "1986-01-02T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
    assert_eq!(format.format_zoned(value).unwrap(), "1986-01-02 002");
}

#[test]
fn provider_accepts_explicit_patterns() {
    use avenger_format::DateTimeFormatProvider;
    let provider = avenger_format_datetime_d3::D3DateTimeFormatProvider;
    let config = D3DateTimeFormatConfig::new().with_timezone("America/New_York");
    let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let instant = date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    for (pattern, civil, zoned) in [
        ("%c", "1/1/2024, 12:00:00 AM", "12/31/2023, 7:00:00 PM"),
        ("%Y-%m-%d", "2024-01-01", "2023-12-31"),
        ("", "", ""),
    ] {
        let prepared = provider.prepare_naive(&config, pattern).unwrap();
        assert_eq!(
            prepared.format(NaiveDateTimeInput::Date(date)).unwrap(),
            civil
        );
        assert_eq!(
            provider
                .prepare_zoned(&config, pattern)
                .unwrap()
                .format(instant)
                .unwrap(),
            zoned
        );
    }
    let config = config.with_timezone("local");
    assert!(provider.prepare_zoned(&config, "%c").is_err());
}

#[test]
fn provider_uses_selected_custom_locale_and_reports_missing_locales() {
    use avenger_format::DateTimeFormatProvider;
    let provider = avenger_format_datetime_d3::D3DateTimeFormatProvider;
    let spec = "%x";
    for (registered, selected) in [("fr-FR", "fr_FR"), ("fr_FR", "fr-FR")] {
        let mut config = D3DateTimeFormatConfig::new().with_locale(selected);
        assert!(provider.prepare_naive(&config, spec).is_err());
        config.locales.insert(
            registered.into(),
            DateTimeLocaleSpec {
                date: "%d~%m~%Y".into(),
                ..Default::default()
            },
        );
        let formatter = provider.prepare_naive(&config, spec).unwrap();
        assert_eq!(
            formatter
                .format(NaiveDateTimeInput::Date(
                    NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()
                ))
                .unwrap(),
            "05~01~2024"
        );
        // An exact custom name takes precedence even when its definition is invalid.
        config.locales.insert(
            selected.into(),
            DateTimeLocaleSpec {
                date: "%x".into(),
                ..Default::default()
            },
        );
        assert!(provider.prepare_naive(&config, spec).is_err());
    }
}
