use avenger_format::DateTimeFormatProvider;
use avenger_format_datetime_chrono::ChronoDateTimeFormatProvider;
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::{America::New_York, Asia::Tokyo, Tz};

#[test]
fn prepared_formatters_retain_settings() {
    let provider = ChronoDateTimeFormatProvider::new()
        .with_locale("POSIX")
        .with_timezone(New_York);
    let date_formatter = provider.prepare_date("%Y-%m-%d").unwrap();
    let naive = provider.prepare_naive("%Y-%m-%d %H:%M").unwrap();
    let zoned = provider.prepare_zoned("%Y-%m-%d %f").unwrap();
    drop(provider);

    let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let zoned_value = DateTime::from_timestamp(1_704_067_200, 123_456_789).unwrap();
    assert_eq!(date_formatter.format(date).unwrap(), "2024-01-01");
    assert_eq!(
        naive.format(date.and_hms_opt(13, 45, 0).unwrap()).unwrap(),
        "2024-01-01 13:45"
    );
    assert_eq!(zoned.format(zoned_value).unwrap(), "2023-12-31 123456789");
}

#[test]
fn date_preparation_validates_fields_after_locale_expansion() {
    let provider = ChronoDateTimeFormatProvider::new();
    let date = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
    let formatter = provider.prepare_date("%x").unwrap();
    assert_eq!(formatter.format(date).unwrap(), "02/29/24");
    assert_eq!(provider.prepare_date("").unwrap().format(date).unwrap(), "");
    for spec in ["%H", "%I", "%M", "%S", "%f", "%p", "%s", "%Z", "%X", "%c"] {
        assert!(provider.prepare_date(spec).is_err(), "{spec}");
    }
}

#[test]
fn preparation_rejects_patterns_for_the_wrong_input_type() {
    let provider = ChronoDateTimeFormatProvider::new();
    for spec in ["%s", "%z", "%+"] {
        assert!(provider.prepare_date(spec).is_err(), "{spec}");
        assert!(provider.prepare_naive(spec).is_err(), "{spec}");
        assert!(provider.prepare_zoned(spec).is_ok(), "{spec}");
    }
    for spec in ["%", "%#z"] {
        assert!(provider.prepare_date(spec).is_err(), "{spec}");
        assert!(provider.prepare_naive(spec).is_err(), "{spec}");
        assert!(provider.prepare_zoned(spec).is_err(), "{spec}");
    }
}

#[cfg(feature = "all-locales")]
#[test]
fn uses_chrono_locales_and_validates_expanded_patterns() {
    let spec = "%B";
    let zoned_value = DateTime::UNIX_EPOCH;
    let value = zoned_value.naive_utc();
    for locale in ["fr-FR", "fr_FR"] {
        let provider = ChronoDateTimeFormatProvider::new().with_locale(locale);
        let date = provider.prepare_date(spec).unwrap();
        let naive = provider.prepare_naive(spec).unwrap();
        let zoned = provider.prepare_zoned(spec).unwrap();
        assert_eq!(date.format(value.date()).unwrap(), "janvier");
        assert_eq!(naive.format(value).unwrap(), "janvier");
        assert_eq!(zoned.format(zoned_value).unwrap(), "janvier");
    }

    let provider = ChronoDateTimeFormatProvider::new().with_locale("en_US");
    let spec = "%c";
    assert!(provider.prepare_naive(spec).is_err());
    assert!(provider.prepare_zoned(spec).is_ok());
}

#[cfg(not(feature = "all-locales"))]
#[test]
fn named_locales_require_the_feature() {
    for name in ["en-US", "en_US", "fr-FR", "fr_FR"] {
        let provider = ChronoDateTimeFormatProvider::new().with_locale(name);
        for error in [
            provider.prepare_date("%B").unwrap_err(),
            provider.prepare_naive("%B").unwrap_err(),
            provider.prepare_zoned("%B").unwrap_err(),
        ] {
            assert!(error
                .to_string()
                .contains("requires the `all-locales` feature"));
        }
    }
}

#[test]
fn display_timezone_preserves_epoch() {
    let provider = ChronoDateTimeFormatProvider::new().with_timezone(Tokyo);
    let formatter = provider.prepare_zoned("%F %R %z %s").unwrap();
    assert_eq!(
        formatter.format(DateTime::UNIX_EPOCH).unwrap(),
        "1970-01-01 09:00 +0900 0"
    );
}

#[test]
fn preparation_rejects_unknown_locales() {
    let provider = ChronoDateTimeFormatProvider::new().with_locale("unknown");
    assert!(provider.prepare_date("%Y").is_err());
    assert!(provider.prepare_naive("%Y").is_err());
    assert!(provider.prepare_zoned("%Y").is_err());
}

#[test]
fn serialized_timezone_defaults_to_utc_and_validates_names() {
    let provider: ChronoDateTimeFormatProvider = serde_json::from_str("{}").unwrap();
    assert_eq!(provider.timezone, Tz::UTC);
    let provider = provider.with_timezone("America/New_York".parse().unwrap());
    let json = serde_json::to_string(&provider).unwrap();
    assert!(json.contains(r#""timezone":"America/New_York""#));
    assert_eq!(
        serde_json::from_str::<ChronoDateTimeFormatProvider>(&json).unwrap(),
        provider
    );
    assert!(
        serde_json::from_str::<ChronoDateTimeFormatProvider>(r#"{"timezone":"invalid/zone"}"#)
            .is_err()
    );
}

#[test]
fn out_of_range_display_dates_return_errors() {
    let provider = ChronoDateTimeFormatProvider::new().with_timezone(Tokyo);
    let formatter = provider.prepare_zoned("%F").unwrap();
    assert!(formatter
        .format(DateTime::<Utc>::MAX_UTC)
        .unwrap_err()
        .to_string()
        .contains("calendar range"));
}
