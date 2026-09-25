use avenger_format::{
    FormattedNumber, NumberFormatConfig, NumberFormatError, NumberFormatProvider,
    NumberFormatRegistry, NumberFormatRequest, PreparedNumberFormatter,
};
use std::sync::Arc;

#[test]
fn unknown_provider_is_rejected() {
    let registry = NumberFormatRegistry::default();
    let config = NumberFormatConfig::new("missing");
    assert!(registry
        .prepare(&config, &NumberFormatRequest::new(""))
        .unwrap_err()
        .to_string()
        .contains("missing"));
}

#[derive(Debug)]
struct Literal(&'static str);
impl NumberFormatProvider for Literal {
    fn prepare(
        &self,
        config: &NumberFormatConfig,
        request: &NumberFormatRequest,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        assert_eq!(config.locale.as_deref(), Some("custom"));
        assert_eq!(request.spec, "provider-specific syntax");
        Ok(Arc::new(Literal(self.0)))
    }
}
impl PreparedNumberFormatter for Literal {
    fn format(&self, _: f64) -> FormattedNumber {
        FormattedNumber::plain(self.0)
    }
}

#[test]
fn providers_are_replaceable_without_changing_prepared_formatters() {
    let mut registry = NumberFormatRegistry::default();
    registry.register("literal", Arc::new(Literal("first")));
    let config = NumberFormatConfig {
        locale: Some("custom".into()),
        ..NumberFormatConfig::new("literal")
    };
    let request = NumberFormatRequest::new("provider-specific syntax");
    let old_registry = registry.clone();
    let prepared = registry.prepare(&config, &request).unwrap();
    registry.register("literal", Arc::new(Literal("second")));
    assert_ne!(registry.cache_id(), old_registry.cache_id());
    assert_eq!(prepared.format(1.0).text, "first");
    assert_eq!(
        old_registry
            .prepare(&config, &request)
            .unwrap()
            .format(1.0)
            .text,
        "first"
    );
    assert_eq!(
        registry
            .prepare(&config, &request)
            .unwrap()
            .format(1.0)
            .text,
        "second"
    );
}

#[test]
fn serialized_configuration_requires_provider_selection() {
    assert!(serde_json::from_str::<NumberFormatConfig>("{}").is_err());
    let config: NumberFormatConfig = serde_json::from_str(r#"{"provider":"custom"}"#).unwrap();
    assert_eq!(config, NumberFormatConfig::new("custom"));
}

#[derive(Debug)]
struct DateLiteral(&'static str);
impl avenger_format::DateTimeFormatProvider for DateLiteral {
    fn prepare_naive(
        &self,
        _: &avenger_format::DateTimeFormatConfig,
        request: &avenger_format::DateTimeFormatRequest,
    ) -> Result<
        Arc<dyn avenger_format::PreparedCivilDateTimeFormatter>,
        avenger_format::DateTimeFormatError,
    > {
        assert_eq!(request.spec, serde_json::json!({"calendar": "custom"}));
        Ok(Arc::new(DateLiteral(self.0)))
    }
    fn prepare_zoned(
        &self,
        _: &avenger_format::DateTimeFormatConfig,
        request: &avenger_format::DateTimeFormatRequest,
    ) -> Result<
        Arc<dyn avenger_format::PreparedInstantFormatter>,
        avenger_format::DateTimeFormatError,
    > {
        assert_eq!(request.spec, serde_json::json!({"calendar": "custom"}));
        Ok(Arc::new(DateLiteral(self.0)))
    }
}
impl avenger_format::PreparedCivilDateTimeFormatter for DateLiteral {
    fn format(
        &self,
        _: avenger_format::NaiveDateTimeInput,
    ) -> Result<String, avenger_format::DateTimeFormatError> {
        Ok(self.0.into())
    }
}
impl avenger_format::PreparedInstantFormatter for DateLiteral {
    fn format(
        &self,
        _: avenger_format::ZonedDateTimeInput,
    ) -> Result<String, avenger_format::DateTimeFormatError> {
        Ok(self.0.into())
    }
}

#[test]
fn datetime_providers_require_selection_and_preserve_registry_snapshots() {
    use avenger_format::{DateTimeFormatConfig, DateTimeFormatRegistry, DateTimeFormatRequest};
    assert!(serde_json::from_str::<DateTimeFormatConfig>("{}").is_err());
    let config: DateTimeFormatConfig = serde_json::from_str(r#"{"provider":"custom"}"#).unwrap();
    assert_eq!(config, DateTimeFormatConfig::new("custom"));
    let request = DateTimeFormatRequest::new(serde_json::json!({"calendar": "custom"}));
    let mut registry = DateTimeFormatRegistry::default();
    assert!(registry
        .prepare_zoned(&config, &request)
        .unwrap_err()
        .to_string()
        .contains("custom"));
    registry.register("custom", Arc::new(DateLiteral("first")));
    let snapshot = registry.clone();
    let prepared = registry.prepare_zoned(&config, &request).unwrap();
    let civil = registry.prepare_naive(&config, &request).unwrap();
    assert_eq!(registry.cache_id(), snapshot.cache_id());
    registry.register("custom", Arc::new(DateLiteral("second")));
    assert_ne!(registry.cache_id(), snapshot.cache_id());
    let instant = chrono::DateTime::UNIX_EPOCH;
    let date = avenger_format::NaiveDateTimeInput::Date(instant.date_naive());
    assert_eq!(civil.format(date).unwrap(), "first");
    assert_eq!(
        registry
            .prepare_naive(&config, &request)
            .unwrap()
            .format(date)
            .unwrap(),
        "second"
    );
    assert_eq!(prepared.format(instant).unwrap(), "first");
    assert_eq!(
        snapshot
            .prepare_zoned(&config, &request)
            .unwrap()
            .format(instant)
            .unwrap(),
        "first"
    );
    assert_eq!(
        registry
            .prepare_zoned(&config, &request)
            .unwrap()
            .format(instant)
            .unwrap(),
        "second"
    );
}
