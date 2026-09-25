use avenger_format::{
    FormattedNumber, NumberFormatConfig, NumberFormatContext, NumberFormatError,
    NumberFormatProvider, NumberFormatRequest, PreparedNumberFormatter,
};
use avenger_format_number::default_number_format_registry;
use serde_json::json;
use std::sync::Arc;

#[test]
fn provider_prepares_d3_labels_with_context_and_overrides() {
    let registry = default_number_format_registry();
    let config = NumberFormatConfig::default();
    for (context, spec, value, expected) in [
        (
            NumberFormatContext::Scalar,
            Some(",.2f"),
            1234.5,
            "1,234.50",
        ),
        (NumberFormatContext::Continuous, None, 0.0012, "0.0012"),
        (NumberFormatContext::Discrete, None, 1234.5, "1234.5"),
        (
            NumberFormatContext::Step {
                step: 100000.0,
                reference_value: 1100000.0,
            },
            Some("s"),
            900000.0,
            "0.9M",
        ),
    ] {
        let request = NumberFormatRequest {
            spec: spec.map(str::to_owned),
            context,
            ..Default::default()
        };
        assert_eq!(
            registry
                .prepare(&config, &request)
                .unwrap()
                .format(value)
                .text,
            expected
        );
    }
    let mut config = NumberFormatConfig {
        locale: Some("custom".into()),
        ..Default::default()
    };
    config.locales.insert(
        "custom".into(),
        json!({"decimal": ",", "thousands": ".", "grouping": [3]}),
    );
    let request = NumberFormatRequest {
        spec: Some("08,.2f".into()),
        options: [
            ("precision".into(), json!(1)),
            ("width".into(), json!(null)),
            ("zero".into(), json!(false)),
        ]
        .into(),
        ..Default::default()
    };
    let mut config: NumberFormatConfig =
        serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert_eq!(
        registry
            .prepare(&config, &request)
            .unwrap()
            .format(1234.5)
            .text,
        "1.234,5"
    );
    for (name, value) in [
        ("precision", json!(-1)),
        ("align", json!("?")),
        ("unknown", json!(true)),
    ] {
        let request = NumberFormatRequest {
            options: [(name.into(), value)].into(),
            ..Default::default()
        };
        assert!(registry.prepare(&config, &request).is_err());
    }
    config.provider = "missing".into();
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
        assert_eq!(request.spec.as_deref(), Some("not a D3 specifier"));
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
    let mut registry = (*default_number_format_registry()).clone();
    registry.register("literal", Arc::new(Literal("first")));
    let config = NumberFormatConfig {
        provider: "literal".into(),
        locale: Some("custom".into()),
        ..Default::default()
    };
    let request = NumberFormatRequest {
        spec: Some("not a D3 specifier".into()),
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
