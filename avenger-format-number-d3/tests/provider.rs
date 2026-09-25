use avenger_format::{NumberFormatConfig, NumberFormatRequest};
use serde_json::json;

#[test]
fn provider_prepares_explicit_d3_specs_and_options() {
    let mut registry = avenger_format::NumberFormatRegistry::default();
    registry.register(
        "d3",
        std::sync::Arc::new(avenger_format_number_d3::D3NumberFormatProvider),
    );
    let config = NumberFormatConfig::new("d3").with_locale("en_US");
    assert_eq!(
        registry
            .prepare(&config, ",.2f")
            .unwrap()
            .format(1234.5)
            .text,
        "1,234.50"
    );
    for (spec, options, value, expected) in [
        (",", json!({"auto_precision":true}), 0.0012, "0.0012"),
        ("", json!({"auto_precision":true}), 1234.5, "1234.5"),
        ("c", json!({}), 1234.5, "1234.5"),
        (
            "s",
            json!({"step":100000.0, "reference_value":1100000.0}),
            900000.0,
            "0.9M",
        ),
        (
            ".2f",
            json!({"step":0.001, "reference_value":1.0}),
            0.125,
            "0.13",
        ),
        (".2f", json!({"auto_precision":true}), 1.0, "1.00"),
    ] {
        let request = NumberFormatRequest {
            options: serde_json::from_value(options).unwrap(),
            ..NumberFormatRequest::new(spec)
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
    for (registered, selected) in [("de-DE", "de_DE"), ("de_DE", "de-DE")] {
        let config = NumberFormatConfig::new("d3")
            .with_locale(selected)
            .with_custom_locale(
                registered,
                json!({"decimal": ",", "thousands": ".", "grouping": [3]}),
            );
        let config: NumberFormatConfig =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(
            registry
                .prepare(&config, ",.1f")
                .unwrap()
                .format(1234.5)
                .text,
            "1.234,5"
        );
        // An exact custom name takes precedence even when its definition is invalid.
        let config = config.with_custom_locale(selected, json!({"grouping": "invalid"}));
        assert!(registry.prepare(&config, ",.1f").is_err());
    }
    for (name, value) in [
        ("precision", json!(1)),
        ("width", json!(5)),
        ("unknown", json!(true)),
        ("auto_precision", json!("yes")),
        ("step", json!(null)),
        ("step", json!(0.1)),
        ("reference_value", json!(1.0)),
    ] {
        let request = NumberFormatRequest {
            options: [(name.into(), value)].into(),
            ..NumberFormatRequest::new("")
        };
        assert!(registry.prepare(&config, &request).is_err());
    }
    let request = NumberFormatRequest {
        options: serde_json::from_value(
            json!({"auto_precision":true, "step":0.1, "reference_value":1.0}),
        )
        .unwrap(),
        ..NumberFormatRequest::new("f")
    };
    assert!(registry.prepare(&config, &request).is_err());
}
