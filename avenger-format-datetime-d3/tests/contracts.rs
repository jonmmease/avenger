use avenger_format_datetime_d3::{
    D3DateTimeFormatProvider, DateTimeFormatContext, DateTimeFormatError, DateTimeLocaleRegistry,
    DateTimeLocaleSpec, DateTimeParseError, PreparedDateTimeFormat, ResolvedDateTimeLocale,
};
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::{America::New_York, Asia::Tokyo, UTC};

#[test]
fn naive_values_preserve_fields() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, New_York);
    let scalar = PreparedDateTimeFormat::new(Some("%Y-%m-%d %H:%M:%S.%L %f"), context).unwrap();
    let date = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
    assert_eq!(
        scalar
            .format_naive(date.and_hms_micro_opt(13, 5, 6, 7_999).unwrap())
            .unwrap(),
        "2024-02-29 13:05:06.007 007000"
    );
}

#[test]
fn date_preparation_rejects_time_fields_including_locale_expansions() {
    use avenger_format::DateTimeFormatProvider;
    for directive in ["%H", "%I", "%M", "%S", "%L", "%f", "%p", "%Q", "%s", "%Z"] {
        let definition = DateTimeLocaleSpec {
            date: directive.into(),
            ..Default::default()
        };
        let locale = ResolvedDateTimeLocale::new("custom", definition.clone()).unwrap();
        let provider = D3DateTimeFormatProvider::new()
            .with_locale("custom")
            .with_custom_locale("custom", definition);
        for spec in [directive, "%x"] {
            assert!(provider
                .prepare_date(spec)
                .unwrap_err()
                .to_string()
                .contains(directive));
            let prepared =
                PreparedDateTimeFormat::new(Some(spec), DateTimeFormatContext::new(&locale, UTC))
                    .unwrap();
            assert!(prepared.format_date(NaiveDate::MIN).is_err());
        }
    }
    let provider = D3DateTimeFormatProvider::new();
    for spec in ["%X", "%c"] {
        assert!(provider.prepare_date(spec).is_err(), "{spec}");
    }
}

#[test]
fn naive_preparation_rejects_zoned_fields_including_locale_expansions() {
    use avenger_format::DateTimeFormatProvider;
    for directive in ["%Z", "%Q", "%s"] {
        let provider = D3DateTimeFormatProvider::new()
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
                .prepare_naive(spec)
                .unwrap_err()
                .to_string()
                .contains(directive));
            assert!(provider.prepare_zoned(spec).is_ok());
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
        registry.resolve("fr-FR").is_ok(),
        cfg!(feature = "all-locales")
    );
    registry
        .register_custom_locale_json("fr-FR", include_str!("../locales/fr-FR.json"))
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
    let date = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
    assert_eq!(original.format_date(date).unwrap(), "janvier 02/01/2024");
    assert_eq!(
        prepare(&registry.resolve("fr-FR").unwrap())
            .format_date(date)
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
            .format_date(date)
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
    let date = NaiveDate::MIN;
    for pattern in ["", "literal é"] {
        assert_eq!(
            PreparedDateTimeFormat::new(Some(pattern), context)
                .unwrap()
                .format_date(date)
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
fn naive_leap_seconds_are_rejected() {
    let locale = ResolvedDateTimeLocale::en_us();
    let context = DateTimeFormatContext::new(&locale, UTC);
    let scalar = PreparedDateTimeFormat::new(Some("%H:%M:%S.%L"), context).unwrap();
    let value = NaiveDate::from_ymd_opt(2016, 12, 31)
        .unwrap()
        .and_hms_milli_opt(23, 59, 59, 1500)
        .unwrap();
    assert_eq!(
        scalar.format_naive(value),
        Err(DateTimeFormatError::LeapSecondForNaive)
    );
}

#[test]
fn submillisecond_zoned_values_use_javascript_date_precision() {
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
    let provider = D3DateTimeFormatProvider::new()
        .with_locale("en_US")
        .with_timezone(New_York);
    let provider: D3DateTimeFormatProvider =
        serde_json::from_str(&serde_json::to_string(&provider).unwrap()).unwrap();
    let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let zoned_value = date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    for (pattern, expected) in [("%Y-%m-%d", "2024-01-01"), ("%x", "1/1/2024"), ("", "")] {
        let prepared = provider.prepare_date(pattern).unwrap();
        assert_eq!(prepared.format(date).unwrap(), expected);
    }
    for (pattern, naive, zoned) in [
        ("%c", "1/1/2024, 12:00:00 AM", "12/31/2023, 7:00:00 PM"),
        ("%Y-%m-%d", "2024-01-01", "2023-12-31"),
        ("", "", ""),
    ] {
        let prepared = provider.prepare_naive(pattern).unwrap();
        assert_eq!(prepared.format(zoned_value.naive_utc()).unwrap(), naive);
        assert_eq!(
            provider
                .prepare_zoned(pattern)
                .unwrap()
                .format(zoned_value)
                .unwrap(),
            zoned
        );
    }
    let defaults: D3DateTimeFormatProvider = serde_json::from_str("{}").unwrap();
    assert_eq!(defaults.timezone, UTC);
    assert!(serde_json::from_str::<D3DateTimeFormatProvider>(r#"{"timezone":"local"}"#).is_err());
}

#[test]
fn provider_uses_selected_custom_locale_and_reports_missing_locales() {
    use avenger_format::DateTimeFormatProvider;
    let spec = "%x";
    for (registered, selected) in [("fr-FR", "fr_FR"), ("fr_FR", "fr-FR")] {
        let mut provider = D3DateTimeFormatProvider::new().with_locale(selected);
        assert_eq!(
            provider.prepare_date(spec).is_ok(),
            cfg!(feature = "all-locales")
        );
        provider.locales.insert(
            registered.into(),
            DateTimeLocaleSpec {
                date: "%d~%m~%Y".into(),
                ..Default::default()
            },
        );
        let formatter = provider.prepare_date(spec).unwrap();
        // An exact custom name takes precedence even when its definition is invalid.
        provider.locales.insert(
            selected.into(),
            DateTimeLocaleSpec {
                date: "%x".into(),
                ..Default::default()
            },
        );
        assert!(provider.prepare_date(spec).is_err());
        drop(provider);
        assert_eq!(
            formatter
                .format(NaiveDate::from_ymd_opt(2024, 1, 5).unwrap())
                .unwrap(),
            "05~01~2024"
        );
    }
    assert!(D3DateTimeFormatProvider::new()
        .with_locale("unknown")
        .prepare_date(spec)
        .is_err());
}

#[cfg(feature = "all-locales")]
#[test]
fn bundled_locales_match_vendored_definitions() {
    use avenger_format::DateTimeFormatProvider;
    let registry = DateTimeLocaleRegistry::with_builtins();
    let value = NaiveDate::from_ymd_opt(2024, 1, 2)
        .unwrap()
        .and_hms_opt(13, 5, 6)
        .unwrap();
    let pattern = "%c | %x | %X | %a %A %b %B %p";
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/locales")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let id = path.file_stem().unwrap().to_str().unwrap();
        let definition: DateTimeLocaleSpec =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let locale = registry.resolve(id).unwrap();
        assert_eq!(locale.definition(), &definition, "{id}");
        let expected =
            PreparedDateTimeFormat::new(Some(pattern), DateTimeFormatContext::new(&locale, UTC))
                .unwrap()
                .format_naive(value)
                .unwrap();
        for name in [id.to_owned(), id.replace('-', "_")] {
            let formatter = D3DateTimeFormatProvider::new()
                .with_locale(&name)
                .prepare_naive(pattern)
                .unwrap();
            assert_eq!(formatter.format(value).unwrap(), expected, "{name}");
        }
    }
}
