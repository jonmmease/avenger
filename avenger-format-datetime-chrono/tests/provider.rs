use avenger_format::{DateTimeFormatConfig, DateTimeFormatRegistry, NaiveDateTimeInput};
use avenger_format_datetime_chrono::ChronoDateTimeFormatProvider;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::json;
use std::sync::Arc;

fn registry() -> DateTimeFormatRegistry {
    let mut registry = DateTimeFormatRegistry::default();
    registry.register("chrono", Arc::new(ChronoDateTimeFormatProvider));
    registry.register("d3", Arc::new(D3DateTimeFormatProvider));
    registry
}

#[test]
fn registry_selects_both_providers_without_changing_the_consumer() {
    let registry = registry();
    let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let instant = DateTime::from_timestamp(1_704_067_200, 123_456_789).unwrap();
    for (provider, locale, fraction) in
        [("chrono", "en-US", "123456789"), ("d3", "en_US", "123000")]
    {
        let config = DateTimeFormatConfig::new(provider)
            .with_locale(locale)
            .with_timezone("America/New_York");
        let civil = registry.prepare_naive(&config, "%Y-%m-%d").unwrap();
        let zoned = registry.prepare_zoned(&config, "%Y-%m-%d").unwrap();
        assert_eq!(
            civil.format(NaiveDateTimeInput::Date(date)).unwrap(),
            "2024-01-01"
        );
        assert_eq!(zoned.format(instant).unwrap(), "2023-12-31");
        let zoned = registry.prepare_zoned(&config, "%f").unwrap();
        assert_eq!(zoned.format(instant).unwrap(), fraction);
    }
}

#[test]
fn preparation_rejects_patterns_for_the_wrong_input_type() {
    let registry = registry();
    let config = DateTimeFormatConfig::new("chrono");
    for spec in ["%s", "%z", "%+"] {
        assert!(registry.prepare_naive(&config, spec).is_err(), "{spec}");
        assert!(registry.prepare_zoned(&config, spec).is_ok(), "{spec}");
    }
    for spec in ["%", "%#z"] {
        assert!(registry.prepare_naive(&config, spec).is_err(), "{spec}");
        assert!(registry.prepare_zoned(&config, spec).is_err(), "{spec}");
    }
}

#[test]
fn uses_chrono_locales_and_validates_expanded_patterns() {
    let registry = registry();
    let spec = "%B";
    let instant = DateTime::UNIX_EPOCH;
    let value = NaiveDateTimeInput::DateTime(instant.naive_utc());
    for locale in ["fr-FR", "fr_FR"] {
        let config = DateTimeFormatConfig::new("chrono").with_locale(locale);
        let civil = registry.prepare_naive(&config, spec).unwrap();
        let zoned = registry.prepare_zoned(&config, spec).unwrap();
        assert_eq!(civil.format(value).unwrap(), "janvier");
        assert_eq!(zoned.format(instant).unwrap(), "janvier");
    }

    let config = DateTimeFormatConfig::new("chrono").with_locale("en_US");
    let spec = "%c";
    assert!(registry.prepare_naive(&config, spec).is_err());
    assert!(registry.prepare_zoned(&config, spec).is_ok());
}

#[test]
fn display_timezone_preserves_epoch() {
    let config = DateTimeFormatConfig::new("chrono").with_timezone("Asia/Tokyo");
    let formatter = registry().prepare_zoned(&config, "%F %R %z %s").unwrap();
    assert_eq!(
        formatter.format(DateTime::UNIX_EPOCH).unwrap(),
        "1970-01-01 09:00 +0900 0"
    );
}

#[test]
fn preparation_rejects_unsupported_configuration() {
    let registry = registry();
    let config = DateTimeFormatConfig::new("chrono");
    for (config, spec) in [
        (config.clone(), json!({})),
        (config.clone().with_locale("unknown"), json!("%Y")),
        (
            config.clone().with_custom_locale("custom", json!({})),
            json!("%Y"),
        ),
    ] {
        assert!(registry.prepare_naive(&config, spec.clone()).is_err());
        assert!(registry.prepare_zoned(&config, spec).is_err());
    }
    let config = config.with_timezone("invalid/zone");
    assert!(registry.prepare_zoned(&config, "%Y").is_err());
}

#[test]
fn out_of_range_display_dates_return_errors() {
    let config = DateTimeFormatConfig::new("chrono").with_timezone("Asia/Tokyo");
    let formatter = registry().prepare_zoned(&config, "%F").unwrap();
    assert!(formatter
        .format(DateTime::<Utc>::MAX_UTC)
        .unwrap_err()
        .to_string()
        .contains("calendar range"));
}
