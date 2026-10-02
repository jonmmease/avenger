use avenger_format::DateTimeFormatProvider;
use avenger_format_datetime_chrono::{ChronoDateTimeFormatConfig, ChronoDateTimeFormatProvider};
use chrono::{DateTime, NaiveDate, Utc};

#[test]
fn prepared_formatters_retain_settings() {
    let provider = ChronoDateTimeFormatProvider;
    let config = ChronoDateTimeFormatConfig::new()
        .with_locale("en-US")
        .with_timezone("America/New_York");
    let date_formatter = provider.prepare_date(&config, "%Y-%m-%d").unwrap();
    let naive = provider.prepare_naive(&config, "%Y-%m-%d %H:%M").unwrap();
    let zoned = provider.prepare_zoned(&config, "%Y-%m-%d %f").unwrap();
    drop(config);

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
    let provider = ChronoDateTimeFormatProvider;
    let config = ChronoDateTimeFormatConfig::new();
    let date = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
    let formatter = provider.prepare_date(&config, "%x").unwrap();
    assert_eq!(formatter.format(date).unwrap(), "02/29/24");
    assert_eq!(
        provider
            .prepare_date(&config, "")
            .unwrap()
            .format(date)
            .unwrap(),
        ""
    );
    for spec in ["%H", "%I", "%M", "%S", "%f", "%p", "%s", "%Z", "%X", "%c"] {
        assert!(provider.prepare_date(&config, spec).is_err(), "{spec}");
    }
}

#[test]
fn preparation_rejects_patterns_for_the_wrong_input_type() {
    let provider = ChronoDateTimeFormatProvider;
    let config = ChronoDateTimeFormatConfig::new();
    for spec in ["%s", "%z", "%+"] {
        assert!(provider.prepare_date(&config, spec).is_err(), "{spec}");
        assert!(provider.prepare_naive(&config, spec).is_err(), "{spec}");
        assert!(provider.prepare_zoned(&config, spec).is_ok(), "{spec}");
    }
    for spec in ["%", "%#z"] {
        assert!(provider.prepare_date(&config, spec).is_err(), "{spec}");
        assert!(provider.prepare_naive(&config, spec).is_err(), "{spec}");
        assert!(provider.prepare_zoned(&config, spec).is_err(), "{spec}");
    }
}

#[test]
fn uses_chrono_locales_and_validates_expanded_patterns() {
    let provider = ChronoDateTimeFormatProvider;
    let spec = "%B";
    let zoned_value = DateTime::UNIX_EPOCH;
    let value = zoned_value.naive_utc();
    for locale in ["fr-FR", "fr_FR"] {
        let config = ChronoDateTimeFormatConfig::new().with_locale(locale);
        let date = provider.prepare_date(&config, spec).unwrap();
        let naive = provider.prepare_naive(&config, spec).unwrap();
        let zoned = provider.prepare_zoned(&config, spec).unwrap();
        assert_eq!(date.format(value.date()).unwrap(), "janvier");
        assert_eq!(naive.format(value).unwrap(), "janvier");
        assert_eq!(zoned.format(zoned_value).unwrap(), "janvier");
    }

    let config = ChronoDateTimeFormatConfig::new().with_locale("en_US");
    let spec = "%c";
    assert!(provider.prepare_naive(&config, spec).is_err());
    assert!(provider.prepare_zoned(&config, spec).is_ok());
}

#[test]
fn display_timezone_preserves_epoch() {
    let config = ChronoDateTimeFormatConfig::new().with_timezone("Asia/Tokyo");
    let formatter = ChronoDateTimeFormatProvider
        .prepare_zoned(&config, "%F %R %z %s")
        .unwrap();
    assert_eq!(
        formatter.format(DateTime::UNIX_EPOCH).unwrap(),
        "1970-01-01 09:00 +0900 0"
    );
}

#[test]
fn preparation_rejects_unknown_locales() {
    let provider = ChronoDateTimeFormatProvider;
    let config = ChronoDateTimeFormatConfig::new().with_locale("unknown");
    assert!(provider.prepare_date(&config, "%Y").is_err());
    assert!(provider.prepare_naive(&config, "%Y").is_err());
    assert!(provider.prepare_zoned(&config, "%Y").is_err());
}

#[test]
fn timezone_validation_applies_only_to_zoned_preparation() {
    let provider = ChronoDateTimeFormatProvider;
    let config = ChronoDateTimeFormatConfig::new().with_timezone("invalid/zone");
    assert!(provider.prepare_date(&config, "%Y").is_ok());
    assert!(provider.prepare_naive(&config, "%Y").is_ok());
    assert!(provider.prepare_zoned(&config, "%Y").is_err());
}

#[test]
fn out_of_range_display_dates_return_errors() {
    let config = ChronoDateTimeFormatConfig::new().with_timezone("Asia/Tokyo");
    let formatter = ChronoDateTimeFormatProvider
        .prepare_zoned(&config, "%F")
        .unwrap();
    assert!(formatter
        .format(DateTime::<Utc>::MAX_UTC)
        .unwrap_err()
        .to_string()
        .contains("calendar range"));
}
