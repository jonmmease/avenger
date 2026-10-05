use avenger_format::{NumberFormatError, NumberFormatProvider};
use avenger_format_number_d3::{D3NumberFormatProvider, D3NumberPrecision, NumberLocaleSpec};

#[test]
fn provider_prepares_explicit_patterns_and_precision() {
    use D3NumberPrecision::{Automatic, FromSpecifier};
    let provider = D3NumberFormatProvider::new().with_locale("en_US");
    for (pattern, precision, value, expected) in [
        (",.2f", FromSpecifier, 1234.5, "1,234.50"),
        (",", Automatic, 0.0012, "0.0012"),
        ("", Automatic, 1234.5, "1234.5"),
        ("c", FromSpecifier, 1234.5, "1234.5"),
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
        let provider = D3NumberFormatProvider::new().with_locale(selected);
        let result = provider.prepare(",.1f");
        if cfg!(feature = "all-locales") {
            assert!(result.is_ok());
        } else {
            assert!(matches!(
                result,
                Err(NumberFormatError::LocaleUnavailable { locale, .. }) if locale == "de-DE"
            ));
        }
        let provider = provider.with_custom_locale(
            registered,
            NumberLocaleSpec {
                decimal: "·".into(),
                thousands: "_".into(),
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
        assert!(matches!(
            provider.prepare(",.1f"),
            Err(NumberFormatError::InvalidLocaleData { .. })
        ));
        drop(provider);
        assert_eq!(formatter.format(1234.5).text, "1_234·5");
    }
    assert!(matches!(
        provider.with_locale("unknown").prepare("f"),
        Err(NumberFormatError::LocaleUnavailable { locale, .. }) if locale == "unknown"
    ));
}

#[test]
fn provider_preserves_pattern_errors_in_each_precision_mode() {
    for precision in [
        D3NumberPrecision::FromSpecifier,
        D3NumberPrecision::Automatic,
    ] {
        let result = D3NumberFormatProvider::new()
            .with_precision(precision)
            .prepare("💠>.2q");
        assert!(matches!(
            result,
            Err(NumberFormatError::InvalidPattern { position: Some(7), message })
                if message.contains("`q`")
        ));
    }
}

#[cfg(feature = "all-locales")]
#[test]
fn bundled_locales_match_vendored_definitions() {
    use avenger_format_number_d3::{NumberLocaleRegistry, PreparedNumberFormat};
    let registry = NumberLocaleRegistry::with_builtins();
    let value = -1234567.89;
    let pattern = "$,.2f";
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/locales")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let id = path.file_stem().unwrap().to_str().unwrap();
        let definition: NumberLocaleSpec =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let locale = registry.resolve(id).unwrap();
        assert_eq!(locale.definition(), &definition, "{id}");
        let expected = PreparedNumberFormat::new(Some(pattern), &locale)
            .unwrap()
            .format(value);
        for name in [id.to_owned(), id.replace('-', "_")] {
            let formatter = D3NumberFormatProvider::new()
                .with_locale(&name)
                .prepare(pattern)
                .unwrap();
            assert_eq!(formatter.format(value), expected, "{name}");
        }
    }
}
