use avenger_format::{
    FormattedNumber, NumberFormatConfig, NumberFormatError, NumberFormatProvider,
    NumberFormatRegistry, NumberFormatRequest, PreparedNumberFormatter,
};
use std::sync::Arc;

#[test]
fn unknown_provider_is_rejected() {
    let registry = NumberFormatRegistry::default();
    let config = NumberFormatConfig {
        provider: "missing".into(),
        ..Default::default()
    };
    assert!(registry
        .prepare(&config, &Default::default())
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
        assert_eq!(request.spec.as_deref(), Some("provider-specific syntax"));
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
        provider: "literal".into(),
        locale: Some("custom".into()),
        ..Default::default()
    };
    let request = NumberFormatRequest {
        spec: Some("provider-specific syntax".into()),
        ..Default::default()
    };
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
