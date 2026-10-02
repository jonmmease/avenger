use avenger_format::NumberFormatProvider;
use avenger_format_number_d3::{D3NumberFormatProvider, D3NumberPrecision, NumberLocaleSpec};

#[test]
fn provider_prepares_explicit_patterns_and_precision() {
    use D3NumberPrecision::{Automatic, FromSpecifier, Step};
    let provider = D3NumberFormatProvider::new().with_locale("en_US");
    for (pattern, precision, value, expected) in [
        (",.2f", FromSpecifier, 1234.5, "1,234.50"),
        (",", Automatic, 0.0012, "0.0012"),
        ("", Automatic, 1234.5, "1234.5"),
        ("c", FromSpecifier, 1234.5, "1234.5"),
        (
            "s",
            Step {
                step: 100000.0,
                reference_value: 1100000.0,
            },
            900000.0,
            "0.9M",
        ),
        (
            ".2f",
            Step {
                step: 0.001,
                reference_value: 1.0,
            },
            0.125,
            "0.13",
        ),
        (".2f", Automatic, 1.0, "1.00"),
    ] {
        let formatter = provider
            .clone()
            .with_precision(precision)
            .prepare(pattern)
            .unwrap();
        assert_eq!(formatter.format(value).text, expected);
    }
    for (registered, selected) in [("de-DE", "de_DE"), ("de_DE", "de-DE")] {
        let provider = D3NumberFormatProvider::new()
            .with_locale(selected)
            .with_custom_locale(
                registered,
                NumberLocaleSpec {
                    decimal: ",".into(),
                    thousands: ".".into(),
                    grouping: vec![3],
                    ..Default::default()
                },
            );
        let provider: D3NumberFormatProvider =
            serde_json::from_str(&serde_json::to_string(&provider).unwrap()).unwrap();
        let formatter = provider.prepare(",.1f").unwrap();
        // An exact custom name takes precedence even when its definition is invalid.
        let provider = provider.with_custom_locale(
            selected,
            NumberLocaleSpec {
                grouping: vec![0],
                ..Default::default()
            },
        );
        assert!(provider.prepare(",.1f").is_err());
        drop(provider);
        assert_eq!(formatter.format(1234.5).text, "1.234,5");
    }
    for (step, reference_value) in [(f64::NAN, 1.0), (0.1, f64::INFINITY)] {
        let provider = provider.clone().with_precision(Step {
            step,
            reference_value,
        });
        assert!(provider.prepare("f").is_err());
    }
    assert!(provider.with_locale("unknown").prepare("f").is_err());
}
