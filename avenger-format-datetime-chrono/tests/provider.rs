use avenger_format::{DateTimeFormatProvider, NaiveDateTimeInput};
use avenger_format_datetime_chrono::{ChronoDateTimeFormatConfig, ChronoDateTimeFormatProvider};
use avenger_format_datetime_d3::{D3DateTimeFormatConfig, D3DateTimeFormatProvider};
use chrono::{DateTime, NaiveDate, Utc};

#[test]
fn typed_providers_share_prepared_formatter_interfaces() {
    fn check<P: DateTimeFormatProvider>(provider: P, config: P::Config, fraction: &str) {
        let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let instant = DateTime::from_timestamp(1_704_067_200, 123_456_789).unwrap();
        let civil = provider.prepare_naive(&config, "%Y-%m-%d").unwrap();
        let zoned = provider.prepare_zoned(&config, "%Y-%m-%d").unwrap();
        assert_eq!(
            civil.format(NaiveDateTimeInput::Date(date)).unwrap(),
            "2024-01-01"
        );
        assert_eq!(zoned.format(instant).unwrap(), "2023-12-31");
        let zoned = provider.prepare_zoned(&config, "%f").unwrap();
        drop(config);
        assert_eq!(zoned.format(instant).unwrap(), fraction);
    }
    check(
        ChronoDateTimeFormatProvider,
        ChronoDateTimeFormatConfig::new()
            .with_locale("en-US")
            .with_timezone("America/New_York"),
        "123456789",
    );
    check(
        D3DateTimeFormatProvider,
        D3DateTimeFormatConfig::new()
            .with_locale("en_US")
            .with_timezone("America/New_York"),
        "123000",
    );
}

#[test]
fn preparation_rejects_patterns_for_the_wrong_input_type() {
    let provider = ChronoDateTimeFormatProvider;
    let config = ChronoDateTimeFormatConfig::new();
    for spec in ["%s", "%z", "%+"] {
        assert!(provider.prepare_naive(&config, spec).is_err(), "{spec}");
        assert!(provider.prepare_zoned(&config, spec).is_ok(), "{spec}");
    }
    for spec in ["%", "%#z"] {
        assert!(provider.prepare_naive(&config, spec).is_err(), "{spec}");
        assert!(provider.prepare_zoned(&config, spec).is_err(), "{spec}");
    }
}

#[test]
fn uses_chrono_locales_and_validates_expanded_patterns() {
    let provider = ChronoDateTimeFormatProvider;
    let spec = "%B";
    let instant = DateTime::UNIX_EPOCH;
    let value = NaiveDateTimeInput::DateTime(instant.naive_utc());
    for locale in ["fr-FR", "fr_FR"] {
        let config = ChronoDateTimeFormatConfig::new().with_locale(locale);
        let civil = provider.prepare_naive(&config, spec).unwrap();
        let zoned = provider.prepare_zoned(&config, spec).unwrap();
        assert_eq!(civil.format(value).unwrap(), "janvier");
        assert_eq!(zoned.format(instant).unwrap(), "janvier");
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
fn preparation_rejects_unknown_locales_and_timezones() {
    let provider = ChronoDateTimeFormatProvider;
    let config = ChronoDateTimeFormatConfig::new().with_locale("unknown");
    assert!(provider.prepare_naive(&config, "%Y").is_err());
    assert!(provider.prepare_zoned(&config, "%Y").is_err());
}

#[test]
fn timezone_validation_applies_only_to_instants() {
    fn check<P: DateTimeFormatProvider>(provider: P, config: P::Config) {
        assert!(provider.prepare_naive(&config, "%Y").is_ok());
        assert!(provider.prepare_zoned(&config, "%Y").is_err());
    }
    check(
        ChronoDateTimeFormatProvider,
        ChronoDateTimeFormatConfig::new().with_timezone("invalid/zone"),
    );
    check(
        D3DateTimeFormatProvider,
        D3DateTimeFormatConfig::new().with_timezone("invalid/zone"),
    );
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
