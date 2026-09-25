use avenger_format_datetime_d3::{
    DateTimeFormatContext, DateTimeFormatError, DateTimeFormatOverrides, DateTimeLocaleRegistry,
    DateTimeLocaleSpec, DateTimeParseError, NaiveDateTimeInput, PreparedDateTimeFormat,
    PreparedTimeMultiFormat, ResolvedDateTimeLocale, TimeMultiFormatSpec,
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::{America::New_York, Asia::Tokyo, UTC};

#[test]
fn civil_values_preserve_fields_and_select_calendar_formats() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, New_York);
    let scalar =
        PreparedDateTimeFormat::new(Some("%Y-%m-%d %H:%M:%S.%L %f"), Default::default(), context)
            .unwrap();
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

    let multi = PreparedTimeMultiFormat::new(&Default::default(), context).unwrap();
    for (value, expected) in [
        ("2024-01-01T00:00:00", "2024"),
        ("2024-04-01T00:00:00", "April"),
        ("2024-05-01T00:00:00", "May"),
        ("2024-05-05T00:00:00", "May 05"),
        ("2024-05-06T00:00:00", "Mon 06"),
        ("2024-05-06T01:00:00", "01 AM"),
        ("2024-05-06T01:02:00", "01:02"),
        ("2024-05-06T01:02:03", ":03"),
        ("2024-05-06T01:02:03.004", ".004"),
    ] {
        assert_eq!(
            multi
                .format_naive(NaiveDateTimeInput::DateTime(value.parse().unwrap()))
                .unwrap(),
            expected,
            "{value}"
        );
    }
}

#[test]
fn civil_patterns_reject_instant_fields_including_locale_expansions() {
    let date = NaiveDateTimeInput::Date(NaiveDate::from_ymd_opt(2024, 3, 10).unwrap());
    for directive in ["%Z", "%Q", "%s"] {
        let locale = ResolvedDateTimeLocale::new(
            "custom",
            DateTimeLocaleSpec {
                time: directive.into(),
                ..Default::default()
            },
        )
        .unwrap();
        let context = DateTimeFormatContext::new(&locale, New_York);
        for pattern in [directive, "%c"] {
            let format =
                PreparedDateTimeFormat::new(Some(pattern), Default::default(), context).unwrap();
            let expected = DateTimeFormatError::TimezoneFieldForNaive(directive.into());
            assert_eq!(format.validate_naive(), Err(expected.clone()));
            assert_eq!(format.format_naive(date), Err(expected));
        }
    }
}

#[test]
fn timezone_overrides_preserve_epoch_and_reject_civil_inputs() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, New_York);
    let overrides = DateTimeFormatOverrides {
        timezone: Some("Asia/Tokyo".into()),
    };
    let format =
        PreparedDateTimeFormat::new(Some("%Y-%m-%d %H:%M %Z %Q %s"), overrides.clone(), context)
            .unwrap();
    assert_eq!(
        format
            .format_zoned(DateTime::from_timestamp_millis(0).unwrap())
            .unwrap(),
        "1970-01-01 09:00 +0900 0 0"
    );
    let civil = PreparedDateTimeFormat::new(Some("%Y"), overrides, context).unwrap();
    assert_eq!(
        civil.format_naive(NaiveDateTimeInput::Date(NaiveDate::MIN)),
        Err(DateTimeFormatError::TimezoneOverrideForNaive)
    );
    assert!(matches!(
        PreparedDateTimeFormat::new(
            None,
            DateTimeFormatOverrides { timezone: Some("invalid/zone".into()) },
            context
        ),
        Err(DateTimeFormatError::InvalidTimezone(zone)) if zone == "invalid/zone"
    ));
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
        PreparedDateTimeFormat::new(
            Some("%B %x"),
            Default::default(),
            DateTimeFormatContext::new(locale, UTC),
        )
        .unwrap()
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
            PreparedDateTimeFormat::new(Some(pattern), Default::default(), context),
            Err(DateTimeFormatError::Parse(DateTimeParseError::Invalid { position, .. }))
                if position == expected_position
        ));
    }
    let date = NaiveDateTimeInput::Date(NaiveDate::MIN);
    for pattern in ["", "literal é"] {
        assert_eq!(
            PreparedDateTimeFormat::new(Some(pattern), Default::default(), context)
                .unwrap()
                .format_naive(date)
                .unwrap(),
            pattern
        );
    }
}

#[test]
fn multi_format_aliases_and_empty_patterns_follow_vega() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, UTC);
    let date = NaiveDateTimeInput::Date(NaiveDate::from_ymd_opt(2024, 5, 6).unwrap());
    for (date_pattern, day_pattern, expected) in [
        (Some("date"), Some("day"), "date"),
        (None, Some("day"), "day"),
        (Some(""), Some("day"), "day"),
        (Some(""), Some(""), "Mon 06"),
    ] {
        let spec = TimeMultiFormatSpec {
            date: date_pattern.map(str::to_owned),
            day: day_pattern.map(str::to_owned),
            ..Default::default()
        };
        assert_eq!(
            PreparedTimeMultiFormat::new(&spec, context)
                .unwrap()
                .format_naive(date)
                .unwrap(),
            expected
        );
    }
    let spec: TimeMultiFormatSpec = serde_json::from_str(r#"{"year":"","quarter":"Q%q"}"#).unwrap();
    let multi = PreparedTimeMultiFormat::new(&spec, context).unwrap();
    for (month, expected) in [(1, "2024"), (4, "Q2")] {
        let value = NaiveDateTimeInput::Date(NaiveDate::from_ymd_opt(2024, month, 1).unwrap());
        assert_eq!(multi.format_naive(value).unwrap(), expected);
    }
}

#[test]
fn multi_format_preflight_checks_inactive_patterns() {
    let locale = ResolvedDateTimeLocale::en_us();
    let spec = TimeMultiFormatSpec {
        year: Some("%Q".into()),
        ..Default::default()
    };
    let multi =
        PreparedTimeMultiFormat::new(&spec, DateTimeFormatContext::new(&locale, UTC)).unwrap();
    let expected = DateTimeFormatError::TimezoneFieldForNaive("%Q".into());
    assert_eq!(multi.validate_naive(), Err(expected.clone()));
    assert_eq!(
        multi.format_naive(NaiveDateTimeInput::Date(
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
        )),
        Err(expected)
    );
    assert_eq!(
        multi
            .format_naive(NaiveDateTimeInput::Date(
                NaiveDate::from_ymd_opt(2024, 5, 1).unwrap()
            ))
            .unwrap(),
        "May"
    );
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
        let scalar =
            PreparedDateTimeFormat::new(Some("%Y-%m-%d"), Default::default(), context).unwrap();
        let multi = PreparedTimeMultiFormat::new(&Default::default(), context).unwrap();
        assert_eq!(
            scalar.format_zoned(value),
            Err(DateTimeFormatError::DateTimeOutOfRange)
        );
        assert_eq!(
            multi.format_zoned(value),
            Err(DateTimeFormatError::DateTimeOutOfRange)
        );
    }
    // The display date fits, but its midnight falls before the earliest UTC date.
    let value = Tokyo
        .from_local_datetime(&NaiveDate::MIN.and_hms_opt(12, 0, 0).unwrap())
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let multi = PreparedTimeMultiFormat::new(
        &Default::default(),
        DateTimeFormatContext::new(&locale, Tokyo),
    )
    .unwrap();
    assert_eq!(
        multi.format_zoned(value),
        Err(DateTimeFormatError::DateTimeOutOfRange)
    );
}

#[test]
fn civil_leap_seconds_are_rejected() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, UTC);
    let scalar =
        PreparedDateTimeFormat::new(Some("%H:%M:%S.%L"), Default::default(), context).unwrap();
    let multi = PreparedTimeMultiFormat::new(&Default::default(), context).unwrap();
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
    assert_eq!(
        multi.format_naive(value),
        Err(DateTimeFormatError::LeapSecondForNaive)
    );
}

#[test]
fn submillisecond_instants_use_javascript_date_precision() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, UTC);
    let format =
        PreparedDateTimeFormat::new(Some("%Q %s %L %f"), Default::default(), context).unwrap();
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
    let multi = PreparedTimeMultiFormat::new(&Default::default(), context).unwrap();
    assert_eq!(multi.format_zoned(just_before_epoch).unwrap(), "1970");
}

#[test]
fn ordinal_uses_calendar_date_after_midnight_offset_change() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, chrono_tz::Asia::Kathmandu);
    let format =
        PreparedDateTimeFormat::new(Some("%Y-%m-%d %j"), Default::default(), context).unwrap();
    // D3's elapsed-day calculation gives 001 after the January 1, 1986 midnight gap.
    let value = "1986-01-02T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
    assert_eq!(format.format_zoned(value).unwrap(), "1986-01-02 002");
}

#[test]
fn provider_accepts_explicit_patterns_and_multi_formats() {
    use avenger_format::{DateTimeFormatConfig, DateTimeFormatProvider, DateTimeFormatRequest};
    use serde_json::json;
    let provider = avenger_format_datetime_d3::D3DateTimeFormatProvider;
    let config = DateTimeFormatConfig {
        timezone: Some("America/New_York".into()),
        ..DateTimeFormatConfig::new("d3")
    };
    let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let instant = date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    for (spec, civil, zoned) in [
        (
            json!("%c"),
            "1/1/2024, 12:00:00 AM",
            "12/31/2023, 7:00:00 PM",
        ),
        (json!("%Y-%m-%d"), "2024-01-01", "2023-12-31"),
        (json!({}), "2024", "07 PM"),
        (
            json!({"year":"year %Y", "hours":"hour %H"}),
            "year 2024",
            "hour 19",
        ),
        (json!(""), "", ""),
    ] {
        let prepared = provider
            .prepare(&config, &DateTimeFormatRequest::new(spec))
            .unwrap();
        assert_eq!(
            prepared
                .format_naive(NaiveDateTimeInput::Date(date))
                .unwrap(),
            civil
        );
        assert_eq!(prepared.format_zoned(instant).unwrap(), zoned);
    }
    for spec in [json!("%c"), json!({})] {
        let prepared = provider
            .prepare(
                &config,
                &DateTimeFormatRequest {
                    timezone: Some("UTC".into()),
                    ..DateTimeFormatRequest::new(spec)
                },
            )
            .unwrap();
        assert!(prepared.validate_naive().is_err());
        assert!(prepared
            .format_naive(NaiveDateTimeInput::Date(date))
            .is_err());
        assert!(prepared.format_zoned(instant).is_ok());
    }
    for request in [
        DateTimeFormatRequest::new(json!(null)),
        DateTimeFormatRequest::new(json!(42)),
        DateTimeFormatRequest {
            options: [("calendar".into(), "unsupported".into())].into(),
            ..DateTimeFormatRequest::new("%c")
        },
        DateTimeFormatRequest {
            timezone: Some("local".into()),
            ..DateTimeFormatRequest::new("%c")
        },
    ] {
        assert!(provider.prepare(&config, &request).is_err());
    }
}

#[test]
fn provider_uses_selected_custom_locale_and_reports_missing_locales() {
    use avenger_format::{DateTimeFormatConfig, DateTimeFormatProvider, DateTimeFormatRequest};
    let provider = avenger_format_datetime_d3::D3DateTimeFormatProvider;
    let mut config = DateTimeFormatConfig {
        locale: Some("custom".into()),
        ..DateTimeFormatConfig::new("d3")
    };
    let request = DateTimeFormatRequest::new("%x");
    assert!(provider.prepare(&config, &request).is_err());
    config.locales.insert(
        "custom".into(),
        serde_json::to_value(avenger_format_datetime_d3::DateTimeLocaleSpec {
            date: "%d~%m~%Y".into(),
            ..Default::default()
        })
        .unwrap(),
    );
    let formatter = provider.prepare(&config, &request).unwrap();
    assert_eq!(
        formatter
            .format_naive(NaiveDateTimeInput::Date(
                NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()
            ))
            .unwrap(),
        "05~01~2024"
    );
    config
        .locales
        .insert("custom".into(), serde_json::json!({"months": []}));
    assert!(provider.prepare(&config, &request).is_err());
}
