use avenger_format::{NumberFormatConfig, NumberFormatContext, NumberFormatRequest};
use avenger_format_number_d3::default_number_format_registry;
use serde_json::json;

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
    let config: NumberFormatConfig =
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
}
